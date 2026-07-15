//! The high-level `Model` facade.
//!
//! `Model` wraps a parsed tree plus diagnostics and exposes typed iterators, an
//! id-lookup index, editing (renumbering), and lossless re-emission. It is the
//! surface downstream crates and the Python bindings consume — owned types, no
//! leaked lifetimes.

use migjorn_syntax::{Diagnostic, GreenTree, Parsed, SyntaxKind};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::cell::{
    cell_id, cell_param_ranges, cell_param_views, cells, parse_cell, parse_param_key, promote_cell,
    scan_cell_params, Cell, CellParam, GeomExpr, OwnedCell,
};
use crate::datacard::{data_cards, DataCard};
use crate::emit::emit_cell;
use crate::material::{materials, parse_material, Material};
use crate::renumber::{
    renumber_cells, renumber_materials, renumber_surfaces, renumber_tallies, renumber_transforms,
    renumber_universes,
};
use crate::surface::{parse_surface, surface_id, surfaces, Surface};
use crate::transform::{parse_transform, transforms, Transform};

/// A parsed MCNP model: the lossless tree, diagnostics, and typed access.
pub struct Model {
    pub(crate) tree: GreenTree,
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
        let Parsed { tree, diagnostics } = migjorn_syntax::parse(src);
        let next_slot = tree.next_slot();
        Model {
            tree,
            diagnostics,
            owned_cells: FxHashMap::default(),
            next_slot,
        }
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

    // --- Stable-handle addressing --------------------------------------------
    //
    // Cards carry a stable *slot* id that survives structural reparses, so a
    // caller (e.g. the Python bindings) can hold a durable handle to a card and
    // resolve it back to a current card index on demand. These methods are the
    // sanctioned way to do that without touching the CST.

    /// Current card index of the card with stable `slot`, or `None` if no live
    /// card has that slot (it was deleted).
    pub fn card_index_of_slot(&self, slot: u32) -> Option<usize> {
        self.tree.card_by_slot(slot)
    }

    /// Stable slot id of the card at `card_index`.
    pub fn slot_at(&self, card_index: usize) -> u32 {
        self.tree.card_slot(card_index)
    }

    /// The exact source text of the card at `card_index`, reflecting any edits
    /// (includes inline `$` comments and continuations).
    pub fn card_source(&self, card_index: usize) -> String {
        self.tree.card_source(card_index)
    }

    /// Slots of every card that parses as a cell, in source order.
    pub fn cell_slots(&self) -> Vec<u32> {
        self.slots_where(|t, i| parse_cell(t, i).is_some())
    }

    /// Slots of every card that parses as a surface, in source order.
    pub fn surface_slots(&self) -> Vec<u32> {
        self.slots_where(|t, i| parse_surface(t, i).is_some())
    }

    /// Slots of every card that parses as an `Mn` material, in source order.
    pub fn material_slots(&self) -> Vec<u32> {
        self.slots_where(|t, i| parse_material(t, i).is_some())
    }

    /// Slots of every card that parses as a `TRn` transform, in source order.
    pub fn transform_slots(&self) -> Vec<u32> {
        self.slots_where(|t, i| parse_transform(t, i).is_some())
    }

    /// Collect the stable slots of cards satisfying `pred`, in source order.
    fn slots_where(&self, pred: impl Fn(&GreenTree, usize) -> bool) -> Vec<u32> {
        (0..self.tree.cards().len())
            .filter(|&i| pred(&self.tree, i))
            .map(|i| self.tree.card_slot(i))
            .collect()
    }

    /// Read the surface with stable `slot`, or `None` if the slot no longer
    /// resolves to a surface card.
    pub fn surface_by_slot(&self, slot: u32) -> Option<Surface> {
        let ci = self.tree.card_by_slot(slot)?;
        parse_surface(&self.tree, ci)
    }

    /// Read the material with stable `slot`, or `None`.
    pub fn material_by_slot(&self, slot: u32) -> Option<Material> {
        let ci = self.tree.card_by_slot(slot)?;
        parse_material(&self.tree, ci)
    }

    /// Read the transform with stable `slot`, or `None`.
    pub fn transform_by_slot(&self, slot: u32) -> Option<Transform> {
        let ci = self.tree.card_by_slot(slot)?;
        parse_transform(&self.tree, ci)
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

    /// Renumber every tally *id* (`Fn` and companion cards) via `map`. The
    /// cell/surface ids inside tally bins are updated by
    /// [`Model::renumber_cells`]/[`Model::renumber_surfaces`], not here.
    pub fn renumber_tallies(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_tallies(&mut self.tree, map);
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

    /// Check the model's consistency and return a list of human-readable
    /// problems (empty when the model is consistent). Two families of problem
    /// are reported, in this order:
    ///
    /// - **Duplicate definitions** — a cell, surface, material, or transform
    ///   number defined by more than one card (each offending number reported
    ///   once, in source order).
    /// - **Dangling references** — every surface, cell, or material a cell
    ///   references must exist; a surface's transformation number must name a
    ///   defined `TRn`, and a periodic surface (a negative transform field) must
    ///   name a defined partner surface.
    ///
    /// This is a semantic check, not a syntactic one: malformed cards surface as
    /// [`Model::diagnostics`] at parse time and are simply skipped here.
    pub fn validate(&self) -> Vec<String> {
        let idx = self.index();
        let mut problems = Vec::new();

        // --- Duplicate definitions ------------------------------------------
        // Ids are read straight from the typed iterators (source order), so a
        // number redefined by a later card is reported exactly once.
        report_duplicates(&mut problems, "cell", self.cells().map(|c| c.id));
        report_duplicates(&mut problems, "surface", self.surfaces().map(|s| s.id));
        report_duplicates(&mut problems, "material", self.materials().map(|m| m.id));
        report_duplicates(&mut problems, "transform", self.transforms().map(|t| t.id));

        // --- Dangling references from cells ---------------------------------
        // Iterate every card position through `read_cell` so structurally-edited
        // (owned) cells are validated against their current, edited state.
        for pos in 0..self.tree.cards().len() {
            let Some(cr) = self.read_cell(pos) else {
                continue;
            };
            let id = cr.id();
            for sid in cr.surface_ids() {
                if idx.surface(sid).is_none() {
                    problems.push(format!("cell {id} references missing surface {sid}"));
                }
            }
            for rid in cr.cell_refs() {
                if idx.cell(rid).is_none() {
                    problems.push(format!("cell {id} references missing cell {rid}"));
                }
            }
            if let Some(m) = cr.material() {
                if m != 0 && idx.material(m).is_none() {
                    problems.push(format!("cell {id} references missing material {m}"));
                }
            }
        }

        // --- Dangling references from surfaces ------------------------------
        // A surface's optional field before the mnemonic is either a positive
        // `TRn` number or, when negative, a periodic partner *surface* number.
        for s in self.surfaces() {
            match s.transform {
                Some(n) if n > 0 => {
                    if idx.transform(n).is_none() {
                        problems.push(format!("surface {} references missing transform {n}", s.id));
                    }
                }
                Some(n) if n < 0 => {
                    let partner = -n;
                    if idx.surface(partner).is_none() {
                        problems.push(format!(
                            "surface {} is periodic with missing surface {partner}",
                            s.id
                        ));
                    }
                }
                _ => {}
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
        let tmp = migjorn_syntax::parse(&cur).tree;
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
        let tmp = migjorn_syntax::parse(&cur).tree;
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
        let Parsed { tree, diagnostics } = migjorn_syntax::parse(new_source);
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
    /// A same-voidness change (real → real, keeping a density) is a pure value
    /// edit that replaces the material token and preserves the card's formatting
    /// byte-for-byte. Crossing the void ↔ non-void boundary also works, but
    /// necessarily adds or removes the density field, so the card is re-emitted
    /// (its own spacing/comments are not preserved; every other card stays
    /// byte-for-byte):
    /// - assigning a non-zero material to a **void** cell gives it a placeholder
    ///   density of `0.0` — set the real value with [`Model::set_cell_density`];
    /// - assigning material `0` makes the cell **void** and drops its density.
    ///
    /// This never fails on voidness, so it is safe to call across a whole cell
    /// loop. A `LIKE n BUT` cell has no material field ([`EditError::NoMaterialField`]),
    /// and a void cell whose geometry did not parse cleanly cannot be re-emitted
    /// ([`EditError::NotEditableGeometry`]).
    ///
    /// Both same-voidness and void-crossing edits are lossless: the material
    /// field (and, when crossing, the density field) is spliced against the
    /// original tokens, so every other byte of the card — geometry spacing,
    /// continuation lines, inline comments, the parameter tail — is preserved.
    pub fn set_cell_material(&mut self, card_index: usize, material: i64) -> Result<(), EditError> {
        let slot = self.tree.card_slot(card_index);
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;
        let mat_tok = cell.material_token.ok_or(EditError::NoMaterialField)?;
        let orig_void = cell.material == Some(0);
        let density_token = cell.density_token;

        if self.owned_cells.contains_key(&slot) {
            // Structurally-edited cell: update the read model, then re-express the
            // header against the original tokens.
            {
                let oc = self.owned_cells.get_mut(&slot).unwrap();
                if oc.material.is_none() {
                    return Err(EditError::NoMaterialField); // a LIKE n BUT cell
                }
                set_owned_material(oc, material);
            }
            if self.tree.card_has_replacement(card_index) {
                self.replace_from_owned(card_index);
            } else {
                self.write_header_emit(slot, mat_tok, orig_void, density_token);
            }
            return Ok(());
        }

        let cur_void = orig_void;
        let new_void = material == 0;
        if cur_void == new_void {
            // Voidness unchanged: rewrite the material token in place.
            self.tree.set_token_int(mat_tok, material);
            return Ok(());
        }
        // Voidness changes: promote for reads (density field is added/removed),
        // then splice the new header in.
        let mut oc = promote_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
        set_owned_material(&mut oc, material);
        self.owned_cells.insert(slot, oc);
        self.write_header_emit(slot, mat_tok, orig_void, density_token);
        Ok(())
    }

    /// Set the density of the cell at `card_index`, in place (positive = atom
    /// density, negative = mass density). A void cell has no density field, so
    /// this returns [`EditError::NoDensityField`]; assign a material first (via
    /// [`Model::set_cell_material`], which makes the cell non-void with a
    /// placeholder density) and then set the real density.
    pub fn set_cell_density(&mut self, card_index: usize, density: f64) -> Result<(), EditError> {
        let slot = self.tree.card_slot(card_index);
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;

        if self.owned_cells.contains_key(&slot) {
            let material = {
                let oc = self.owned_cells.get_mut(&slot).unwrap();
                if oc.material == Some(0) || oc.density.is_none() {
                    return Err(EditError::NoDensityField);
                }
                oc.density = Some(density);
                oc.material.unwrap_or(0)
            };
            if self.tree.card_has_replacement(card_index) {
                self.replace_from_owned(card_index);
            } else if let Some(dt) = cell.density_token {
                // Originally-real cell: rewrite its density token in place.
                self.tree.set_token_text(dt, format!("{density}"));
            } else {
                // Void-originated cell: the density lives in the spliced header.
                let mat_tok = cell.material_token.ok_or(EditError::NoDensityField)?;
                self.tree
                    .set_insertion_before(mat_tok, format!("{material} {density}"));
            }
            return Ok(());
        }
        let tok = cell.density_token.ok_or(EditError::NoDensityField)?;
        self.tree.set_token_text(tok, format!("{density}"));
        Ok(())
    }

    /// Set the transform number of the surface at `card_index`, in place. The
    /// sign of `transform` encodes periodicity (negative = periodic). Adding a
    /// transform to a surface that has none (or removing an existing one) is a
    /// lossless splice: the number is inserted after the id / deleted with its
    /// separator, and every other byte of the card is preserved.
    pub fn set_surface_transform(
        &mut self,
        card_index: usize,
        transform: Option<i64>,
    ) -> Result<(), EditError> {
        let s = parse_surface(&self.tree, card_index).ok_or(EditError::NotASurface)?;
        match (s.transform_token, transform) {
            (Some(tok), Some(v)) => self.tree.set_token_int(tok, v),
            (None, None) => {}
            (Some(tok), None) => {
                // Remove the transform: delete it and one preceding separator.
                self.tree.delete_token(tok);
                if tok > 0 && self.tree.token_kind(tok - 1) == SyntaxKind::WHITESPACE {
                    self.tree.delete_token(tok - 1);
                }
            }
            (None, Some(v)) => {
                // Insert the transform number between the id and the mnemonic.
                self.tree.insert_after(s.id_token, format!(" {v}"));
            }
        }
        Ok(())
    }

    /// Set the `i`-th coefficient of the surface at `card_index`, in place.
    /// `i` indexes [`Surface::coeffs`]; an out-of-range index returns
    /// [`EditError::IndexOutOfRange`].
    pub fn set_surface_coeff(
        &mut self,
        card_index: usize,
        i: usize,
        value: f64,
    ) -> Result<(), EditError> {
        let s = parse_surface(&self.tree, card_index).ok_or(EditError::NotASurface)?;
        let tok = *s.coeff_tokens.get(i).ok_or(EditError::IndexOutOfRange)?;
        self.tree.set_token_text(tok, format!("{value}"));
        Ok(())
    }

    /// Set the fraction of the `entry`-th `ZAID fraction` pair of the material
    /// at `card_index`, in place (positive = atom fraction, negative = weight).
    pub fn set_material_fraction(
        &mut self,
        card_index: usize,
        entry: usize,
        value: f64,
    ) -> Result<(), EditError> {
        let m = parse_material(&self.tree, card_index).ok_or(EditError::NotAMaterial)?;
        let e = m.entries.get(entry).ok_or(EditError::IndexOutOfRange)?;
        self.tree
            .set_token_text(e.fraction_token, format!("{value}"));
        Ok(())
    }

    /// Set the ZAID of the `entry`-th pair of the material at `card_index`, in
    /// place (e.g. `"1001.31c"`). The text is written verbatim.
    pub fn set_material_zaid(
        &mut self,
        card_index: usize,
        entry: usize,
        zaid: &str,
    ) -> Result<(), EditError> {
        let m = parse_material(&self.tree, card_index).ok_or(EditError::NotAMaterial)?;
        let e = m.entries.get(entry).ok_or(EditError::IndexOutOfRange)?;
        self.tree.set_token_text(e.zaid_token, zaid.to_string());
        Ok(())
    }

    /// Set the displacement vector of the transform at `card_index`, in place.
    /// Components already written are rewritten; components that defaulted to 0
    /// (and so have no token) are spliced in after the last present displacement
    /// value — a lossless edit. Missing components are always trailing, so this
    /// never disturbs rotation entries (which require a full displacement).
    pub fn set_transform_displacement(
        &mut self,
        card_index: usize,
        displacement: [f64; 3],
    ) -> Result<(), EditError> {
        let t = parse_transform(&self.tree, card_index).ok_or(EditError::NotATransform)?;
        let present = t
            .displacement_tokens
            .iter()
            .take_while(|x| x.is_some())
            .count();
        for (slot, &v) in t
            .displacement_tokens
            .iter()
            .zip(&displacement)
            .take(present)
        {
            self.tree.set_token_text(slot.unwrap(), format!("{v}"));
        }
        if present < 3 {
            let anchor = if present > 0 {
                t.displacement_tokens[present - 1].unwrap()
            } else {
                t.id_token
            };
            let mut s = String::new();
            for &v in &displacement[present..] {
                s.push_str(&format!(" {v}"));
            }
            self.tree.insert_after(anchor, s);
        }
        Ok(())
    }

    /// Set the rotation entries of the transform at `card_index`, in place.
    /// Shared entries are rewritten; extra entries are spliced in after the last
    /// numeric token, and surplus entries are deleted — all lossless. Adding
    /// rotation to a transform whose displacement is incomplete is ambiguous
    /// (the new numbers would read as displacement) → [`EditError::StructuralEdit`].
    pub fn set_transform_rotation(
        &mut self,
        card_index: usize,
        rotation: &[f64],
    ) -> Result<(), EditError> {
        let t = parse_transform(&self.tree, card_index).ok_or(EditError::NotATransform)?;
        let old = t.rotation_tokens.len();
        let new = rotation.len();
        for (&tok, &v) in t.rotation_tokens.iter().zip(rotation) {
            self.tree.set_token_text(tok, format!("{v}"));
        }
        if new < old {
            for i in new..old {
                let tok = t.rotation_tokens[i];
                self.tree.delete_token(tok);
                if tok > 0 && self.tree.token_kind(tok - 1) == SyntaxKind::WHITESPACE {
                    self.tree.delete_token(tok - 1);
                }
            }
        } else if new > old {
            let present_disp = t
                .displacement_tokens
                .iter()
                .take_while(|x| x.is_some())
                .count();
            let anchor = if old > 0 {
                t.rotation_tokens[old - 1]
            } else if present_disp == 3 {
                t.displacement_tokens[2].unwrap()
            } else {
                return Err(EditError::StructuralEdit);
            };
            let mut s = String::new();
            for &v in &rotation[old..] {
                s.push_str(&format!(" {v}"));
            }
            self.tree.insert_after(anchor, s);
        }
        Ok(())
    }

    /// Intersect the cell's geometry with a signed surface (`id` magnitude,
    /// `negative` sense). The new sense is spliced in after the last geometry
    /// token, so the card's formatting — continuation lines, inline comments,
    /// the parameter tail — is preserved byte-for-byte; only the added surface
    /// appears.
    pub fn add_cell_surface(
        &mut self,
        card_index: usize,
        id: i64,
        negative: bool,
    ) -> Result<(), EditError> {
        let ref_text = if negative {
            format!("-{id}")
        } else {
            id.to_string()
        };
        self.splice_add(card_index, &ref_text, |g| g.intersect_surface(id, negative))
    }

    /// Intersect the cell's geometry with a `#n` cell complement. Formatting is
    /// preserved as in [`Model::add_cell_surface`].
    pub fn add_cell_complement(&mut self, card_index: usize, id: i64) -> Result<(), EditError> {
        let ref_text = format!("#{id}");
        self.splice_add(card_index, &ref_text, |g| g.intersect_cell_complement(id))
    }

    /// Remove every surface reference of magnitude `id` from the cell's
    /// geometry. Returns whether anything was removed. Refuses (with
    /// [`EditError::WouldEmptyGeometry`]) an edit that would leave the cell with
    /// no geometry.
    pub fn remove_cell_surface(&mut self, card_index: usize, id: i64) -> Result<bool, EditError> {
        if let Some(res) = self.try_splice_remove(card_index, id, true) {
            return res;
        }
        self.remove_geometry(card_index, &move |g| g.remove_surface(id))
    }

    /// Remove every `#n` complement of cell `id` from the geometry.
    pub fn remove_cell_complement(
        &mut self,
        card_index: usize,
        id: i64,
    ) -> Result<bool, EditError> {
        if let Some(res) = self.try_splice_remove(card_index, id, false) {
            return res;
        }
        self.remove_geometry(card_index, &move |g| g.remove_cell_complement(id))
    }

    /// Try to remove surface `id` (or the `#id` complement when `is_surface` is
    /// false) as a lossless token splice. Eligible only when the geometry is
    /// *flat* — a top-level intersection or single factor with no unions or
    /// parentheses — and the cell has not already been structurally edited (so
    /// the CST tokens still mirror the logical geometry). Deletes each matching
    /// leaf's tokens plus one separator, preserving all other bytes.
    ///
    /// Returns `None` to fall back to the whole-card re-emit (nested unions,
    /// parenthesised regions, or an already-edited cell); `Some(Ok(false))` when
    /// nothing matched; `Some(Err(_))` when the removal would empty the cell.
    fn try_splice_remove(
        &mut self,
        card_index: usize,
        id: i64,
        is_surface: bool,
    ) -> Option<Result<bool, EditError>> {
        let slot = self.tree.card_slot(card_index);
        if self.owned_cells.contains_key(&slot) {
            return None; // a prior structural edit owns this card; keep it consistent
        }
        let cell = parse_cell(&self.tree, card_index)?;
        if !cell.well_formed {
            return None;
        }
        let geometry = cell.geometry.as_ref()?;
        let first = cell.geom_first_token?;
        let last = cell.geom_last_token?;
        // Flatness: bail on any union or parenthesis in the geometry region.
        for t in first..=last {
            match self.tree.token_kind(t) {
                SyntaxKind::COLON | SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => return None,
                _ => {}
            }
        }
        // Determine the tokens to delete: the (first, last) CST span of each
        // matching leaf. A surface is one NUMBER; a `#id` complement is the HASH
        // plus the NUMBER, which we require to be adjacent (else fall back).
        let mut spans: Vec<(u32, u32)> = Vec::new();
        if is_surface {
            geometry.for_each_surface_ref(&mut |s| {
                if s.id == id {
                    spans.push((s.token, s.token));
                }
            });
        } else {
            let mut ok = true;
            geometry.for_each_cell_ref(&mut |c| {
                if c.id == id {
                    if c.token > first && self.tree.token_kind(c.token - 1) == SyntaxKind::HASH {
                        spans.push((c.token - 1, c.token));
                    } else {
                        ok = false;
                    }
                }
            });
            if !ok {
                return None;
            }
        }
        if spans.is_empty() {
            return Some(Ok(false));
        }
        // Guard against emptying the geometry on a trial copy.
        let mut trial = geometry.clone();
        let removed = if is_surface {
            trial.remove_surface(id)
        } else {
            trial.remove_cell_complement(id)
        };
        debug_assert!(removed);
        if trial.is_empty() {
            return Some(Err(EditError::WouldEmptyGeometry));
        }
        // Apply the deletions, each with one separator (prefer the preceding
        // whitespace; for the first geometry leaf take the following one so no
        // leading separator is left behind).
        for &(ls, le) in &spans {
            for t in ls..=le {
                self.tree.delete_token(t);
            }
            if ls > first {
                self.tree.delete_token(ls - 1);
            } else {
                self.tree.delete_token(le + 1);
            }
        }
        // Promote for reads and mirror the removal in the owned model (the tree
        // deletions are emit-only, so `promote_cell` still sees full geometry).
        let mut oc = promote_cell(&self.tree, card_index)?;
        if is_surface {
            oc.geometry.remove_surface(id);
        } else {
            oc.geometry.remove_cell_complement(id);
        }
        self.owned_cells.insert(slot, oc);
        Some(Ok(true))
    }

    /// Intersect a cell's geometry with a new term (`ref_text`, e.g. `-5` or
    /// `#7`) as a lossless token splice, keeping the owned read model in sync.
    ///
    /// The term is emitted after the last geometry token (before the
    /// geometry→parameter trivia), so continuation lines and the parameter tail
    /// stay byte-for-byte. If the current top-level region is a union, the
    /// existing geometry is wrapped in parentheses first so the new intersection
    /// term does not bind inside the union (`a:b` + `-3` → `(a:b) -3`).
    fn splice_add(
        &mut self,
        card_index: usize,
        ref_text: &str,
        mutate: impl FnOnce(&mut GeomExpr),
    ) -> Result<(), EditError> {
        let slot = self.tree.card_slot(card_index);
        if self.tree.card_has_replacement(card_index) {
            // Card is already emitted wholesale; keep it in that (lossy) mode.
            return self.edit_geometry(card_index, mutate);
        }
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
        if !cell.well_formed {
            return Err(EditError::NotEditableGeometry);
        }
        let first = cell
            .geom_first_token
            .ok_or(EditError::NotEditableGeometry)?;
        let last = cell.geom_last_token.ok_or(EditError::NotEditableGeometry)?;
        // Whether the *current* logical region is a top-level union decides
        // parenthesisation. Prefer the owned view (reflects earlier edits).
        let is_union = match self.owned_cells.get(&slot) {
            Some(oc) => matches!(oc.geometry, GeomExpr::Union(_)),
            None => matches!(cell.geometry, Some(GeomExpr::Union(_))),
        };
        if is_union {
            self.tree.insert_before(first, "(");
            self.tree.insert_after(last, format!(") {ref_text}"));
        } else {
            self.tree.insert_after(last, format!(" {ref_text}"));
        }
        // Keep the owned read model consistent (promote on first structural edit;
        // reads go through it, emit goes through the splice above).
        if !self.owned_cells.contains_key(&slot) {
            let oc = promote_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
            self.owned_cells.insert(slot, oc);
        }
        mutate(&mut self.owned_cells.get_mut(&slot).unwrap().geometry);
        Ok(())
    }

    /// Read every cell parameter (`imp:n`, `vol`, `fill`, `trcl`, …) of the cell
    /// at `card_index` as [`CellParam`] views, in source order. Empty for a
    /// non-cell, a `LIKE n BUT` cell, or a cell with no parameter section.
    pub fn cell_params(&self, card_index: usize) -> Vec<CellParam> {
        // Read from the owned tail only when the card is fully in replace mode
        // (emitted from `emit_cell`); a card merely promoted for geometry reads
        // still emits its parameters from the CST, overrides and all — so match
        // whatever `add`/`remove`/`set_cell_param` write to.
        if self.tree.card_has_replacement(card_index) {
            let slot = self.tree.card_slot(card_index);
            if let Some(oc) = self.owned_cells.get(&slot) {
                return params_from_text(&oc.params_text);
            }
        }
        cell_param_views(&self.tree, card_index)
    }

    /// Read the first parameter matching `key` (`"vol"`, or a particle-qualified
    /// `"imp:n"`; see [`Model::remove_cell_param`] for the match rules), or `None`.
    pub fn cell_param(&self, card_index: usize, key: &str) -> Option<CellParam> {
        let (want_key, want_particle) = parse_param_key(key);
        self.cell_params(card_index).into_iter().find(|v| {
            v.key == want_key
                && want_particle
                    .as_deref()
                    .is_none_or(|p| v.particle.as_deref() == Some(p))
        })
    }

    /// Rewrite the value of the first parameter matching `key` in place (e.g.
    /// `set_cell_param(ci, "imp:n", "2")`), returning whether one matched. The
    /// keyword and its position are untouched — only the value is replaced — so
    /// unlike remove-then-add the parameter order is preserved and the rest of
    /// the card stays byte-for-byte. See [`Model::remove_cell_param`] for the
    /// `key`/`imp:n` match rules.
    pub fn set_cell_param(
        &mut self,
        card_index: usize,
        key: &str,
        value: &str,
    ) -> Result<bool, EditError> {
        let card = *self
            .tree
            .cards()
            .get(card_index)
            .ok_or(EditError::NotACell)?;
        if card.kind != SyntaxKind::CELL_CARD {
            return Err(EditError::NotACell);
        }
        let (want_key, want_particle) = parse_param_key(key);
        if self.tree.card_has_replacement(card_index) {
            // Replace-mode card: rewrite the value in the owned tail string.
            let slot = self.tree.card_slot(card_index);
            let oc = self.owned_cells.get_mut(&slot).unwrap();
            match set_param_in_text(&oc.params_text, &want_key, want_particle.as_deref(), value) {
                Some(rebuilt) => {
                    oc.params_text = rebuilt;
                    self.replace_from_owned(card_index);
                    return Ok(true);
                }
                None => return Ok(false),
            }
        }
        // CST-backed card: locate the matching entry's value tokens.
        let mut target = None;
        scan_cell_params(&self.tree, card_index, |p| {
            if target.is_some() || !p.matches(&self.tree, &want_key, want_particle.as_deref()) {
                return;
            }
            target = Some((p.value_tokens.first().copied(), p.end));
        });
        let Some((first, end)) = target else {
            return Ok(false);
        };
        match first {
            // Overwrite the first value token and delete every token after it,
            // through the end of the value region (trivia included), collapsing
            // the value to `value` while leaving the `keyword=` prefix intact.
            Some(first) => {
                self.tree.set_token_text(first, value.to_string());
                for t in (first + 1)..=end {
                    self.tree.delete_token(t);
                }
            }
            // A value-less entry (`key=` with nothing after): splice the value in
            // right after the `=`.
            None => self.tree.insert_after(end, value.to_string()),
        }
        Ok(true)
    }

    /// Append a parameter to a cell's parameter section (`text`, e.g.
    /// `"imp:n=1"` or `"u=5"`). It is spliced in after the cell's last token and
    /// before any trailing inline `$` comment, so the rest of the card — geometry
    /// spacing, continuation lines, existing parameters — stays byte-for-byte.
    /// `text` must be non-empty and single-line.
    pub fn add_cell_param(&mut self, card_index: usize, text: &str) -> Result<(), EditError> {
        let text = text.trim();
        if text.is_empty() || text.contains(['\n', '\r']) {
            return Err(EditError::InvalidCardText);
        }
        let card = *self
            .tree
            .cards()
            .get(card_index)
            .ok_or(EditError::NotACell)?;
        if card.kind != SyntaxKind::CELL_CARD {
            return Err(EditError::NotACell);
        }
        let last = self
            .tree
            .card_content_tokens(&card)
            .last()
            .ok_or(EditError::NotACell)?;
        if self.tree.card_has_replacement(card_index) {
            // Replace-mode card: fold into the owned tail and re-emit.
            let slot = self.tree.card_slot(card_index);
            let oc = self.owned_cells.get_mut(&slot).unwrap();
            if oc.params_text.is_empty() {
                oc.params_text = text.to_string();
            } else {
                oc.params_text.push(' ');
                oc.params_text.push_str(text);
            }
            self.replace_from_owned(card_index);
        } else {
            self.tree.insert_after(last, format!(" {text}"));
        }
        Ok(())
    }

    /// Remove the first parameter matching `key`. A bare keyword (`"imp"`) matches
    /// the first entry with that keyword regardless of designator; a qualified key
    /// (`"imp:n"`) matches only that particle, so `imp:n` and `imp:p` on one cell
    /// are individually removable. Case-insensitive. Returns whether one was
    /// removed. Lossless for a cell that has not been structurally re-emitted.
    pub fn remove_cell_param(&mut self, card_index: usize, key: &str) -> Result<bool, EditError> {
        let (want_key, want_particle) = parse_param_key(key);
        let params = cell_param_ranges(&self.tree, card_index);
        let Some(p) = params
            .iter()
            .find(|p| p.matches(&want_key, want_particle.as_deref()))
        else {
            return Ok(false);
        };
        let (start, end) = (p.start_token, p.end_token);
        if self.tree.card_has_replacement(card_index) {
            // Replace-mode card: rebuild the owned tail without this parameter.
            let slot = self.tree.card_slot(card_index);
            let oc = self.owned_cells.get_mut(&slot).unwrap();
            if let Some(rebuilt) =
                remove_param_from_text(&oc.params_text, &want_key, want_particle.as_deref())
            {
                oc.params_text = rebuilt;
                self.replace_from_owned(card_index);
            }
            return Ok(true);
        }
        for t in start..=end {
            self.tree.delete_token(t);
        }
        // Drop one preceding whitespace separator (params always follow geometry,
        // so there is one) to avoid leaving a doubled space.
        let card = self.tree.cards()[card_index];
        if start > card.first_tok && self.tree.token_kind(start - 1) == SyntaxKind::WHITESPACE {
            self.tree.delete_token(start - 1);
        }
        Ok(true)
    }

    /// Ensure the cell is promoted, apply `f` to its geometry, and re-emit the
    /// whole card body (a lossy fallback; used only where a lossless splice is
    /// not available, e.g. removing a surface from a nested union).
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
        self.replace_from_owned(card_index);
        Ok(r)
    }

    /// Re-emit the whole card body from the owned read model (the lossy
    /// fallback). Clears any token splices on the card first so the splice and
    /// replacement emit mechanisms never overlap on the same card.
    fn replace_from_owned(&mut self, card_index: usize) {
        let slot = self.tree.card_slot(card_index);
        // Before dropping the token splices, fold any spliced parameters into the
        // owned tail (only meaningful the first time the card enters replace mode;
        // afterwards the splices are already gone).
        if !self.tree.card_has_replacement(card_index) {
            if let Some(cell) = parse_cell(&self.tree, card_index) {
                let card = self.tree.cards()[card_index];
                if let Some(last) = self.tree.card_content_tokens(&card).last() {
                    let pt = self.tree.params_effective_text(cell.params_start, last);
                    if let Some(oc) = self.owned_cells.get_mut(&slot) {
                        oc.params_text = pt;
                    }
                }
            }
        }
        self.tree.clear_card_overlay(card_index);
        let text = emit_cell(&self.owned_cells[&slot]);
        self.tree.replace_card_content(card_index, text);
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
        self.owned_cells.get_mut(&slot).unwrap().geometry = trial;
        self.replace_from_owned(card_index);
        Ok(true)
    }
}

/// Append a problem to `problems` for each id in `ids` that appears more than
/// once. Each duplicated id is reported a single time, at the point its first
/// repeat is seen, so the output is deterministic and free of noise when an id
/// is defined three or more times.
fn report_duplicates(problems: &mut Vec<String>, kind: &str, ids: impl Iterator<Item = i64>) {
    let mut seen = FxHashSet::default();
    let mut reported = FxHashSet::default();
    for id in ids {
        // `seen.insert` is false on a repeat; `reported.insert` then gates the
        // message to the first repeat only.
        if !seen.insert(id) && reported.insert(id) {
            problems.push(format!("duplicate {kind} {id} defined more than once"));
        }
    }
}

/// A structural change applied by [`Model::reparse_with_change`].
enum Change {
    /// A card was inserted at this position in the reparsed tree.
    Insert(usize),
    /// The card at this position was removed.
    Delete(usize),
}

impl Model {
    /// Re-express a structurally-edited cell's header (`material [density]`) on
    /// the CST overlay, from the owned read model against the *original* tokens.
    ///
    /// When the new voidness matches the original card's, the material token is
    /// rewritten in place (the density token, if any, is left untouched so an
    /// unrelated material change never reformats it). When voidness differs from
    /// the original, the original material/density value tokens are deleted and a
    /// single `material [density]` string is spliced in their place. Idempotent:
    /// any prior header splice is cleared first, so repeated edits compose.
    fn write_header_emit(
        &mut self,
        slot: u32,
        mat_tok: u32,
        orig_void: bool,
        density_token: Option<u32>,
    ) {
        let (material, density) = {
            let oc = &self.owned_cells[&slot];
            (oc.material.unwrap_or(0), oc.density)
        };
        let new_void = material == 0;

        // Clear any header splice from an earlier voidness change so this is a
        // clean re-expression.
        self.tree.clear_insertion_before(mat_tok);
        self.tree.undelete_token(mat_tok);
        if let Some(dt) = density_token {
            for t in (mat_tok + 1)..=dt {
                self.tree.undelete_token(t);
            }
        }

        if orig_void == new_void {
            // Same shape as the original card: a plain value rewrite.
            self.tree.set_token_int(mat_tok, material);
        } else {
            // Voidness differs: replace the original header value tokens wholesale.
            let text = if new_void {
                "0".to_string()
            } else {
                format!("{} {}", material, density.unwrap_or(0.0))
            };
            self.tree.set_insertion_before(mat_tok, text);
            self.tree.delete_token(mat_tok);
            if let Some(dt) = density_token {
                for t in (mat_tok + 1)..=dt {
                    self.tree.delete_token(t);
                }
            }
        }
    }
}

/// The wrapper header prepended to an opaque replace-mode parameter tail so it
/// re-parses as the parameter section of a cell. Its byte length is the offset
/// to subtract to map wrapped token spans back onto `params_text`.
const PARAM_TAIL_PREFIX: &str = "t\n1 0 -1 ";

/// Re-parse an opaque replace-mode parameter tail in a minimal wrapper cell,
/// returning the parsed tree and the wrapper cell's card index. The tail is an
/// opaque string only for structurally re-emitted cells; every other path reads
/// parameters straight off the model's CST.
fn parse_param_tail(params_text: &str) -> Option<(GreenTree, usize)> {
    let wrapped = format!("{PARAM_TAIL_PREFIX}{params_text}\n\n1 PX 0\n\nm1 1001 1\n");
    let tree = migjorn_syntax::parse(wrapped).tree;
    let ci = (0..tree.cards().len()).find(|&i| tree.cards()[i].kind == SyntaxKind::CELL_CARD)?;
    Some((tree, ci))
}

/// Read the parameters of an opaque replace-mode tail as public [`CellParam`]
/// views.
fn params_from_text(params_text: &str) -> Vec<CellParam> {
    match parse_param_tail(params_text) {
        Some((tree, ci)) => cell_param_views(&tree, ci),
        None => Vec::new(),
    }
}

/// Remove the parameter matching the uppercased `(key, particle)` from a raw
/// parameter tail, returning the rebuilt tail — or `None` if it is not present.
/// Used only for the rare replace-mode path, where the tail is an opaque string;
/// it re-parses the tail in a minimal wrapper to locate the entry's byte span.
fn remove_param_from_text(params_text: &str, key: &str, particle: Option<&str>) -> Option<String> {
    let (tree, ci) = parse_param_tail(params_text)?;
    let params = cell_param_ranges(&tree, ci);
    let p = params.iter().find(|p| p.matches(key, particle))?;
    let base = PARAM_TAIL_PREFIX.len();
    let s = tree.token_span(p.start_token).start as usize - base;
    let e = tree.token_span(p.end_token).end as usize - base;
    // Drop one preceding space separator if present.
    let cut = if s > 0 && params_text.as_bytes()[s - 1] == b' ' {
        s - 1
    } else {
        s
    };
    let mut out = String::with_capacity(params_text.len());
    out.push_str(&params_text[..cut]);
    out.push_str(&params_text[e..]);
    Some(out.trim().to_string())
}

/// Rewrite the value of the parameter matching the uppercased `(key, particle)`
/// in a raw parameter tail, returning the rebuilt tail — or `None` if it is not
/// present. The replace-mode counterpart of the CST value splice in
/// [`Model::set_cell_param`].
fn set_param_in_text(
    params_text: &str,
    key: &str,
    particle: Option<&str>,
    value: &str,
) -> Option<String> {
    let (tree, ci) = parse_param_tail(params_text)?;
    let base = PARAM_TAIL_PREFIX.len();
    let mut span = None;
    scan_cell_params(&tree, ci, |p| {
        if span.is_some() || !p.matches(&tree, key, particle) {
            return;
        }
        // Byte range of the value region: from the first value token to the last,
        // or the empty gap just after the entry's `=` when it has no value.
        let (s, e) = match (p.value_tokens.first(), p.value_tokens.last()) {
            (Some(&f), Some(&l)) => (
                tree.token_span(f).start as usize - base,
                tree.token_span(l).end as usize - base,
            ),
            _ => {
                let at = tree.token_span(p.end).end as usize - base;
                (at, at)
            }
        };
        span = Some((s, e));
    });
    let (s, e) = span?;
    let mut out = String::with_capacity(params_text.len() - (e - s) + value.len());
    out.push_str(&params_text[..s]);
    out.push_str(value);
    out.push_str(&params_text[e..]);
    Some(out)
}

/// Set an owned cell's material, keeping its density field consistent with
/// MCNP's rule that only non-void cells carry a density: a cell becoming
/// non-void gains a placeholder density of `0.0` (the caller sets the real value
/// separately), and a cell becoming void drops its density.
fn set_owned_material(oc: &mut OwnedCell, material: i64) {
    oc.material = Some(material);
    if material == 0 {
        oc.density = None;
    } else if oc.density.is_none() {
        oc.density = Some(0.0);
    }
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
    let tree = migjorn_syntax::parse(wrapped).tree;
    let of_kind: Vec<usize> = (0..tree.cards().len())
        .filter(|&i| tree.cards()[i].kind == kind)
        .collect();
    if of_kind.len() != 1 {
        return Err(EditError::InvalidCardText);
    }
    let i = of_kind[0];
    // A typed projection returns `Some` even for malformed-but-recognizable
    // input (empty/garbage geometry, non-numeric coefficients, a dangling
    // ZAID), flagging it via `well_formed`. Reject anything not well-formed so
    // we never splice a card that would not round-trip through its typed view.
    let ok = match kind {
        SyntaxKind::CELL_CARD => parse_cell(&tree, i).is_some_and(|c| c.well_formed),
        SyntaxKind::SURFACE_CARD => parse_surface(&tree, i).is_some_and(|s| s.well_formed),
        // Materials must parse as such; other data cards are accepted as-is.
        SyntaxKind::DATA_CARD => {
            let looks_material = text.trim_start().to_ascii_uppercase().starts_with('M')
                && !text.trim_start().to_ascii_uppercase().starts_with("MODE");
            !looks_material || parse_material(&tree, i).is_some_and(|m| m.well_formed)
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

/// Why a value edit could not be applied. Most describe edits that would
/// require inserting or removing tokens (a structural change), which the
/// current overlay-based editing cannot express; they are not data errors.
///
/// `#[non_exhaustive]`: new failure modes may be added, so downstream matches
/// should include a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditError {
    /// The target card is not a (well-formed) cell.
    NotACell,
    /// The target card is not a (well-formed) surface.
    NotASurface,
    /// The target card is not a (well-formed) `Mn` material.
    NotAMaterial,
    /// The target card is not a (well-formed) `TRn` transform.
    NotATransform,
    /// The cell has no material field to set (e.g. a `LIKE n BUT` cell).
    NoMaterialField,
    /// The cell has no density field to set (a void cell); assign a material
    /// first (which makes it non-void), then set the density.
    NoDensityField,
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
    /// The edit is structurally ambiguous and cannot be expressed as a local
    /// splice — currently only adding rotation entries to a transform whose
    /// displacement is incomplete (the new numbers would read as displacement).
    StructuralEdit,
    /// A coefficient/entry index was out of range for the target card.
    IndexOutOfRange,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            EditError::NotACell => "card is not a cell",
            EditError::NotASurface => "card is not a surface",
            EditError::NotAMaterial => "card is not a material",
            EditError::NotATransform => "card is not a transform",
            EditError::NoMaterialField => "cell has no material field to set (a LIKE n BUT cell?)",
            EditError::NoDensityField => {
                "cell has no density field to set (a void cell); assign a material \
                 first (making it non-void), then set the density"
            }
            EditError::NotEditableGeometry => {
                "cell has no editable geometry (a LIKE n BUT cell, or malformed geometry)"
            }
            EditError::WouldEmptyGeometry => "edit would leave the cell with no geometry",
            EditError::InvalidCardText => {
                "text did not parse as exactly one card of the expected kind"
            }
            EditError::NoBlock => "the target block has no card to append after",
            EditError::StructuralEdit => {
                "edit is structurally ambiguous (adding rotation entries to a \
                 transform whose displacement is incomplete)"
            }
            EditError::IndexOutOfRange => "coefficient/entry index is out of range",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for EditError {}

/// Id → card-index lookup maps for a model. Build once, query O(1).
///
/// The maps are private so the public API does not leak the internal hasher;
/// query them through the accessor methods ([`ModelIndex::cell`], `surface`,
/// `material`, `transform`) and iterate ids via [`ModelIndex::cell_ids`] etc.
#[derive(Debug, Default, Clone)]
pub struct ModelIndex {
    /// Cell number → `tree.cards()` index.
    cells: FxHashMap<i64, usize>,
    /// Surface number → `tree.cards()` index.
    surfaces: FxHashMap<i64, usize>,
    /// Material number → `tree.cards()` index.
    materials: FxHashMap<i64, usize>,
    /// Transform number → `tree.cards()` index.
    transforms: FxHashMap<i64, usize>,
}

impl ModelIndex {
    /// Card index of the cell numbered `id`, or `None`.
    pub fn cell(&self, id: i64) -> Option<usize> {
        self.cells.get(&id).copied()
    }
    /// Card index of the surface numbered `id`, or `None`.
    pub fn surface(&self, id: i64) -> Option<usize> {
        self.surfaces.get(&id).copied()
    }
    /// Card index of the material numbered `id`, or `None`.
    pub fn material(&self, id: i64) -> Option<usize> {
        self.materials.get(&id).copied()
    }
    /// Card index of the transform numbered `id`, or `None`.
    pub fn transform(&self, id: i64) -> Option<usize> {
        self.transforms.get(&id).copied()
    }

    /// Number of indexed cells.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
    /// Number of indexed surfaces.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }
    /// Number of indexed materials.
    pub fn material_count(&self) -> usize {
        self.materials.len()
    }
    /// Number of indexed transforms.
    pub fn transform_count(&self) -> usize {
        self.transforms.len()
    }

    /// Iterate all indexed cell numbers (unordered).
    pub fn cell_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.cells.keys().copied()
    }
    /// Iterate all indexed surface numbers (unordered).
    pub fn surface_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.surfaces.keys().copied()
    }
    /// Iterate all indexed material numbers (unordered).
    pub fn material_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.materials.keys().copied()
    }
    /// Iterate all indexed transform numbers (unordered).
    pub fn transform_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.transforms.keys().copied()
    }

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
        assert_eq!(idx.cell_count(), 2);
        assert!(idx.cell(1).is_some());
        assert!(idx.cell(2).is_some());
        assert!(idx.surface(1).is_some());
        assert!(idx.material(1).is_some());
        assert!(idx.transform(1).is_some());
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
        m.index().cell(id).unwrap()
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
    fn assign_material_to_void_cell_adds_placeholder_density() {
        let mut m = Model::parse(MODEL);
        let void_ci = cell_pos(&m, 2); // "2 0 1 imp:n=0"
                                       // Assigning a real material to a void cell just works (no error) and
                                       // gives it a placeholder density of 0.
        m.set_cell_material(void_ci, 5).unwrap();
        let read = m.read_cell(void_ci).unwrap();
        assert!(!read.is_void());
        assert_eq!(read.density(), Some(0.0));
        let out = m.to_source();
        assert!(out.contains("2 5 0 1 imp:n=0"), "got: {out}"); // params preserved
                                                                // Everything else is byte-for-byte.
        assert!(out.contains("1 1 -1.0 -1 imp:n=1"));
        assert!(out.contains("1 SO 5"));
        // The real density can now be set the usual way.
        m.set_cell_density(void_ci, -2.5).unwrap();
        assert!(m.to_source().contains("2 5 -2.5 1 imp:n=0"));
    }

    #[test]
    fn material_zero_makes_cell_void_and_drops_density() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1); // "1 1 -1.0 -1 imp:n=1"
        m.set_cell_material(ci, 0).unwrap();
        assert!(m.read_cell(ci).unwrap().is_void());
        let out = m.to_source();
        assert!(out.contains("1 0 -1 imp:n=1"), "got: {out}");
        // Re-parse is clean.
        assert!(Model::parse(&out).diagnostics().is_empty());
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
        // Lossless: the untouched density `-1.0` is preserved (not reformatted).
        assert!(out.contains("1 1 -1.0 -1 -5 imp:n=1"), "got: {out}");
        // Other cards untouched.
        assert!(out.contains("2 0 1 imp:n=0"));
        assert!(out.contains("1 SO 5"));
        // Re-parse is clean.
        assert!(Model::parse(&out).diagnostics().is_empty());
    }

    #[test]
    fn remove_surface_and_guard_against_emptying() {
        let mut m = Model::parse("t\n1 0 -1 2 imp:n=1\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        assert!(m.remove_cell_surface(ci, 2).unwrap());
        assert!(m.to_source().contains("1 0 -1 imp:n=1"));
        // Removing the last surface is refused.
        assert_eq!(
            m.remove_cell_surface(ci, 1),
            Err(EditError::WouldEmptyGeometry)
        );
    }

    #[test]
    fn void_to_real_preserves_continuation_lines() {
        // The reported bug: a void cell whose params sit on a continuation line.
        // Giving it a material+density must not pull `imp`/`u` onto the geom line.
        let src = "title\n\
                   1 0      -1 -2 -3\n\
                   \x20         imp:n=1.0   u=7\n\
                   \x20         $ a comment\n\
                   \n1 SO 5\n2 PX 0\n3 PY 0\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        // The two-step property flow (set material, then density) is lossless.
        m.set_cell_material(ci, 200).unwrap();
        m.set_cell_density(ci, 2.2875).unwrap();
        let out = m.to_source();
        let expected = "title\n\
                        1 200 2.2875      -1 -2 -3\n\
                        \x20         imp:n=1.0   u=7\n\
                        \x20         $ a comment\n\
                        \n1 SO 5\n2 PX 0\n3 PY 0\n\nm1 1001 1\n";
        assert_eq!(out, expected, "got:\n{out}");
        // Read model reflects the edit.
        let read = m.read_cell(ci).unwrap();
        assert_eq!(read.material(), Some(200));
        assert_eq!(read.density(), Some(2.2875));
    }

    #[test]
    fn add_surface_preserves_multiline_and_inline_comment() {
        let src = "title\n\
                   10 5 -2.0 -1 2  $ inner\n\
                   \x20         -3 4\n\
                   \n1 PX 0\n2 PY 0\n3 PZ 0\n4 SO 9\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(10).unwrap();
        m.add_cell_surface(ci, 99, true).unwrap();
        let out = m.to_source();
        // Multi-line geometry, the inline `$ inner` comment, and the density
        // `-2.0` are all preserved; the new `-99` lands after the last geom token.
        let expected = "title\n\
                        10 5 -2.0 -1 2  $ inner\n\
                        \x20         -3 4 -99\n\
                        \n1 PX 0\n2 PY 0\n3 PZ 0\n4 SO 9\n\nm1 1001 1\n";
        assert_eq!(out, expected, "got:\n{out}");
    }

    #[test]
    fn add_surface_into_union_parenthesises() {
        let mut m = Model::parse("t\n1 0 -1 : 2 imp:n=1\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        m.add_cell_surface(ci, 3, true).unwrap();
        assert!(
            m.to_source().contains("1 0 (-1 : 2) -3 imp:n=1"),
            "got: {}",
            m.to_source()
        );
        // A second add composes without double-wrapping.
        m.add_cell_surface(ci, 4, false).unwrap();
        assert!(
            m.to_source().contains("1 0 (-1 : 2) -3 4 imp:n=1"),
            "got: {}",
            m.to_source()
        );
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn add_cell_param_is_lossless_and_before_trailing_comment() {
        let src = "title\n\
                   1 1 -1.0 -1 imp:n=1  $ fuel\n\
                   \n1 SO 5\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        m.add_cell_param(ci, "u=5").unwrap();
        // New param lands after `imp:n=1` and before the inline `$ fuel` comment.
        assert_eq!(
            m.to_source(),
            "title\n1 1 -1.0 -1 imp:n=1 u=5  $ fuel\n\n1 SO 5\n\nm1 1001 1\n",
            "got:\n{}",
            m.to_source()
        );
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn add_cell_param_to_cell_without_params() {
        let mut m = Model::parse("t\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        m.add_cell_param(ci, "imp:n=1").unwrap();
        assert_eq!(m.to_source(), "t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        assert_eq!(m.add_cell_param(ci, ""), Err(EditError::InvalidCardText));
    }

    #[test]
    fn remove_cell_param_is_lossless() {
        let src = "title\n1 1 -1.0 -1 imp:n=1 u=5 vol=3  $ c\n\n1 SO 5\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        // Remove a middle param.
        assert!(m.remove_cell_param(ci, "u").unwrap());
        assert_eq!(
            m.to_source(),
            "title\n1 1 -1.0 -1 imp:n=1 vol=3  $ c\n\n1 SO 5\n\nm1 1001 1\n",
            "got:\n{}",
            m.to_source()
        );
        // Case-insensitive keyword, ignoring the :particle designator.
        assert!(m.remove_cell_param(ci, "IMP").unwrap());
        assert!(
            m.to_source().contains("1 1 -1.0 -1 vol=3  $ c"),
            "got: {}",
            m.to_source()
        );
        // A missing keyword is a no-op false.
        assert!(!m.remove_cell_param(ci, "fill").unwrap());
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn cell_params_reads_all_params() {
        let m = Model::parse(
            "t\n1 1 -1.0 -1 imp:n=1 imp:p=0 vol=3 *fill=7 (0 0 5)  $ c\n\n1 SO 5\n\nm1 1001 1\n",
        );
        let ci = m.index().cell(1).unwrap();
        let params = m.cell_params(ci);
        assert_eq!(
            params,
            vec![
                CellParam {
                    key: "IMP".into(),
                    particle: Some("N".into()),
                    starred: false,
                    value: "1".into(),
                },
                CellParam {
                    key: "IMP".into(),
                    particle: Some("P".into()),
                    starred: false,
                    value: "0".into(),
                },
                CellParam {
                    key: "VOL".into(),
                    particle: None,
                    starred: false,
                    value: "3".into(),
                },
                CellParam {
                    key: "FILL".into(),
                    particle: None,
                    starred: true,
                    value: "7 ( 0 0 5 )".into(),
                },
            ]
        );
    }

    #[test]
    fn cell_param_is_particle_qualified() {
        let m = Model::parse("t\n1 0 -1 imp:n=1 imp:p=0\n\n1 SO 5\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        // A bare keyword takes the first matching entry.
        assert_eq!(m.cell_param(ci, "imp").unwrap().value, "1");
        // A qualified key selects the specific particle.
        assert_eq!(m.cell_param(ci, "imp:p").unwrap().value, "0");
        assert_eq!(m.cell_param(ci, "imp:n").unwrap().value, "1");
        assert!(m.cell_param(ci, "vol").is_none());
    }

    #[test]
    fn set_cell_param_rewrites_value_in_place() {
        let src = "title\n1 1 -1.0 -1 imp:n=1 vol=3  $ c\n\n1 SO 5\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        assert!(m.set_cell_param(ci, "imp:n", "2").unwrap());
        // Value changes in place; order and every other byte are preserved.
        assert_eq!(
            m.to_source(),
            "title\n1 1 -1.0 -1 imp:n=2 vol=3  $ c\n\n1 SO 5\n\nm1 1001 1\n",
            "got:\n{}",
            m.to_source()
        );
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn set_cell_param_collapses_multi_token_value() {
        let src = "t\n1 0 -1 fill=7 (0 0 5) vol=3\n\n1 SO 5\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        assert!(m.set_cell_param(ci, "fill", "8 (1 0 0)").unwrap());
        assert_eq!(
            m.to_source(),
            "t\n1 0 -1 fill=8 (1 0 0) vol=3\n\n1 SO 5\n\nm1 1001 1\n",
            "got:\n{}",
            m.to_source()
        );
        // A missing parameter is a no-op false.
        assert!(!m.set_cell_param(ci, "trcl", "9").unwrap());
    }

    #[test]
    fn remove_cell_param_particle_qualified() {
        let src = "t\n1 0 -1 imp:n=1 imp:p=0 vol=3\n\n1 SO 5\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(1).unwrap();
        // A qualified key removes only that particle's entry.
        assert!(m.remove_cell_param(ci, "imp:p").unwrap());
        assert_eq!(
            m.to_source(),
            "t\n1 0 -1 imp:n=1 vol=3\n\n1 SO 5\n\nm1 1001 1\n",
            "got:\n{}",
            m.to_source()
        );
        assert!(m.cell_param(ci, "imp:n").is_some());
    }

    #[test]
    fn cell_params_read_and_set_after_splice_promote() {
        // A flat geometry removal promotes the cell for reads but keeps emitting
        // from the CST (no replacement); parameters must still read/write through
        // the tree, consistent with `add`/`remove`/`set_cell_param`.
        let mut m = Model::parse("t\n1 0 -1 2 imp:n=1 vol=3\n\n1 SO 5\n2 SO 9\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        m.remove_cell_surface(ci, 2).unwrap();
        assert_eq!(m.cell_param(ci, "imp:n").unwrap().value, "1");
        assert!(m.set_cell_param(ci, "vol", "42").unwrap());
        assert!(m.to_source().contains("vol=42"), "got:\n{}", m.to_source());
        assert_eq!(m.cell_param(ci, "vol").unwrap().value, "42");
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn cell_params_read_and_set_in_replace_mode() {
        // A union geometry removal forces a whole-card re-emit (replace mode),
        // where the parameter tail is an opaque owned string; exercise that path.
        let mut m =
            Model::parse("t\n1 0 (-1:2) 3 imp:n=1 vol=3\n\n1 SO 5\n2 SO 9\n3 SO 7\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        m.remove_cell_surface(ci, 3).unwrap();
        assert!(m.tree.card_has_replacement(ci), "expected replace mode");
        assert_eq!(m.cell_param(ci, "imp:n").unwrap().value, "1");
        assert!(m.set_cell_param(ci, "vol", "42").unwrap());
        assert_eq!(m.cell_param(ci, "vol").unwrap().value, "42");
        assert!(m.to_source().contains("vol=42"), "got:\n{}", m.to_source());
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn remove_surface_flat_is_lossless() {
        // Flat intersection across a continuation line, with an inline comment.
        let src = "title\n\
                   10 5 -2.0 -1 2 3  $ region\n\
                   \x20         4 imp:n=1\n\
                   \n1 PX 0\n2 PY 0\n3 PZ 0\n4 SO 9\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        let ci = m.index().cell(10).unwrap();
        assert!(m.remove_cell_surface(ci, 2).unwrap());
        // Only surface `2` (and its separator) is gone; everything else stays.
        let expected = "title\n\
                        10 5 -2.0 -1 3  $ region\n\
                        \x20         4 imp:n=1\n\
                        \n1 PX 0\n2 PY 0\n3 PZ 0\n4 SO 9\n\nm1 1001 1\n";
        assert_eq!(m.to_source(), expected, "got:\n{}", m.to_source());
        assert_eq!(m.read_cell(ci).unwrap().signed_surfaces(), vec![-1, 3, 4]);
    }

    #[test]
    fn remove_first_surface_leaves_no_leading_separator() {
        let mut m = Model::parse("t\n1 0 -1 2 3 imp:n=1\n\n1 SO 5\n2 PX 0\n3 PY 0\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        assert!(m.remove_cell_surface(ci, 1).unwrap());
        assert!(
            m.to_source().contains("1 0 2 3 imp:n=1"),
            "got: {}",
            m.to_source()
        );
    }

    #[test]
    fn remove_complement_flat_is_lossless() {
        let mut m = Model::parse(
            "t\n1 0 -1 #7 2 imp:n=1\n7 0 -3 imp:n=1\n\n1 SO 5\n2 PX 0\n3 PZ 0\n\nm1 1001 1\n",
        );
        let ci = m.index().cell(1).unwrap();
        assert!(m.remove_cell_complement(ci, 7).unwrap());
        assert!(
            m.to_source().contains("1 0 -1 2 imp:n=1"),
            "got: {}",
            m.to_source()
        );
        assert_eq!(m.read_cell(ci).unwrap().cell_refs(), Vec::<i64>::new());
        assert!(Model::parse(m.to_source()).diagnostics().is_empty());
    }

    #[test]
    fn remove_from_union_falls_back_but_stays_correct() {
        // A union isn't splice-eligible; the fallback re-emits the card (lossy
        // formatting) but the result is correct and re-parses cleanly.
        let mut m =
            Model::parse("t\n1 0 (-1 : 2) -3 imp:n=1\n\n1 SO 5\n2 PX 0\n3 PY 0\n\nm1 1001 1\n");
        let ci = m.index().cell(1).unwrap();
        assert!(m.remove_cell_surface(ci, 3).unwrap());
        assert_eq!(m.read_cell(ci).unwrap().signed_surfaces(), vec![-1, 2]);
        let out = m.to_source();
        assert!(out.contains("imp:n=1"), "params kept: {out}");
        assert!(Model::parse(&out).diagnostics().is_empty());
    }

    #[test]
    fn material_edit_after_geometry_edit_is_consistent() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1);
        m.add_cell_surface(ci, 9, false).unwrap(); // promotes the cell
        m.set_cell_material(ci, 3).unwrap(); // must route through the owned node
        let out = m.to_source();
        // Lossless: only the material changes; density `-1.0` stays byte-for-byte.
        assert!(out.contains("1 3 -1.0 -1 9 imp:n=1"), "got: {out}");
        assert_eq!(m.read_cell(ci).unwrap().material(), Some(3));
    }

    #[test]
    fn geometry_edit_refused_on_like_cell() {
        let mut m = Model::parse("t\n1 0 -1\n9 LIKE 1 BUT imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let ci = m.index().cell(9).unwrap();
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
        let ci = m.index().cell(10).unwrap();
        assert_eq!(m.slot_at(ci), slot);
        m.add_cell_surface(ci, 1, false).unwrap(); // surface 1 exists in MODEL
        assert_eq!(m.read_cell(ci).unwrap().signed_surfaces(), vec![-1, 1]);
        // Lossless splice: the density `-2.0` of the freshly-added cell is kept.
        assert!(
            m.to_source().contains("10 1 -2.0 -1 1 imp:n=1"),
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
    fn add_rejects_malformed_but_recognizable_cards() {
        // These parse into a typed view but with `well_formed == false`; they
        // must be rejected rather than spliced in (see validate_snippet).
        let mut m = Model::parse(MODEL);
        assert_eq!(m.add_cell("10 0"), Err(EditError::InvalidCardText)); // no geometry
        assert_eq!(m.add_cell("10 0 -5 :"), Err(EditError::InvalidCardText)); // dangling op
        assert_eq!(m.add_surface("10 SO abc"), Err(EditError::InvalidCardText)); // bad coeff
        assert_eq!(m.add_data_card("m7 1001"), Err(EditError::InvalidCardText));
        // no fraction
    }

    #[test]
    fn remove_cell_preserves_other_cards_and_slots() {
        let mut m = Model::parse(MODEL);
        // Hold a handle-slot for cell 1 before removing cell 2.
        let ci1 = m.index().cell(1).unwrap();
        let slot1 = m.slot_at(ci1);
        assert!(m.remove_cell(2).unwrap());
        let out = m.to_source();
        assert!(!out.contains("\n2 0 1"), "cell 2 not removed:\n{out}");
        assert!(out.contains("1 SO 5"), "surfaces intact");
        // Cell 1's slot still resolves to cell 1 (handle survived).
        let pos = m.card_index_of_slot(slot1).unwrap();
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

    #[test]
    fn validate_flags_missing_surface_and_material() {
        // Cell 1 references surface 9 (undefined) and material 5 (undefined).
        let m = Model::parse("t\n1 5 -1.0 -9 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let problems = m.validate();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("cell 1 references missing surface 9")),
            "got: {problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("cell 1 references missing material 5")),
            "got: {problems:?}"
        );
    }

    #[test]
    fn validate_flags_duplicate_definitions() {
        // Cell 1, surface 1, material 1, and transform 1 are each defined twice.
        let m = Model::parse(
            "t\n\
             1 0 -1 imp:n=1\n\
             1 0 -2 imp:n=1\n\
             \n1 SO 5\n1 SO 9\n2 PX 0\n\
             \nm1 1001 1\nm1 8016 1\ntr1 0 0 0\ntr1 1 1 1\n",
        );
        let problems = m.validate();
        for kind in ["cell", "surface", "material", "transform"] {
            let want = format!("duplicate {kind} 1 defined more than once");
            assert!(problems.contains(&want), "missing {want:?} in {problems:?}");
        }
        // A thrice-defined id is still reported exactly once.
        let m = Model::parse("t\n1 0 -1\n\n5 SO 1\n5 SO 2\n5 SO 3\n\nm1 1001 1\n");
        let dupes: Vec<_> = m
            .validate()
            .into_iter()
            .filter(|p| p.contains("duplicate surface 5"))
            .collect();
        assert_eq!(dupes.len(), 1, "got: {dupes:?}");
    }

    #[test]
    fn validate_flags_missing_transform_and_periodic_partner() {
        // Surface 1 uses transform 9 (undefined); surface 2 is periodic with
        // surface 8 (undefined). Surface 3 uses transform 4, which exists.
        let m =
            Model::parse("t\n1 0 -1\n\n1 9 SO 5\n2 -8 PX 0\n3 4 PY 0\n\nm1 1001 1\ntr4 0 0 0\n");
        let problems = m.validate();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("surface 1 references missing transform 9")),
            "got: {problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("surface 2 is periodic with missing surface 8")),
            "got: {problems:?}"
        );
        // The valid transform reference on surface 3 raises nothing.
        assert!(
            !problems.iter().any(|p| p.contains("surface 3")),
            "got: {problems:?}"
        );
    }

    #[test]
    fn set_surface_coeff_and_transform_in_place() {
        let mut m = Model::parse("t\n1 0 -1\n\n1 3 SO 5\n2 SO 9\n\nm1 1001 1\ntr3 0 0 0\n");
        let si = m.index().surface(1).unwrap();
        m.set_surface_coeff(si, 0, 7.5).unwrap();
        assert!(m.to_source().contains("1 3 SO 7.5"), "{}", m.to_source());
        // Out-of-range coefficient index is rejected.
        assert_eq!(
            m.set_surface_coeff(si, 9, 1.0),
            Err(EditError::IndexOutOfRange)
        );
        // An existing transform can be changed in place.
        m.set_surface_transform(si, Some(13)).unwrap();
        assert!(m.to_source().contains("1 13 SO 7.5"), "{}", m.to_source());
        // An existing transform can be removed (lossless splice delete).
        m.set_surface_transform(si, None).unwrap();
        assert!(m.to_source().contains("1 SO 7.5"), "{}", m.to_source());
        // Adding a transform to a surface that has none is a lossless splice.
        let s2 = m.index().surface(2).unwrap();
        m.set_surface_transform(s2, Some(3)).unwrap();
        assert!(m.to_source().contains("2 3 SO 9"), "{}", m.to_source());
    }

    #[test]
    fn set_material_fraction_and_zaid_in_place() {
        let mut m = Model::parse("t\n1 0 -1\n\n1 SO 5\n\nm1 1001.31c 0.5 8016.31c 0.5\n");
        let mi = m.index().material(1).unwrap();
        m.set_material_fraction(mi, 0, 0.7).unwrap();
        m.set_material_zaid(mi, 1, "8016.70c").unwrap();
        assert!(
            m.to_source().contains("m1 1001.31c 0.7 8016.70c 0.5"),
            "{}",
            m.to_source()
        );
        assert_eq!(
            m.set_material_fraction(mi, 5, 1.0),
            Err(EditError::IndexOutOfRange)
        );
    }

    #[test]
    fn set_transform_displacement_and_rotation_in_place() {
        let mut m = Model::parse(
            "t\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n*tr2 0 0 0 1 0 0 0 1 0 0 0 1\n",
        );
        let t1 = m.index().transform(1).unwrap();
        m.set_transform_displacement(t1, [1.0, 2.0, 3.0]).unwrap();
        assert!(m.to_source().contains("tr1 1 2 3"), "{}", m.to_source());
        // A full-displacement transform can gain rotation entries (splice).
        m.set_transform_rotation(t1, &[90.0]).unwrap();
        assert!(m.to_source().contains("tr1 1 2 3 90"), "{}", m.to_source());
        // A same-arity rotation rewrite succeeds (tr2 has 9 entries).
        let t2 = m.index().transform(2).unwrap();
        m.set_transform_rotation(t2, &[2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0])
            .unwrap();
        assert!(
            m.to_source().contains("0 0 0 2 0 0 0 2 0 0 0 2"),
            "{}",
            m.to_source()
        );
        // Surplus rotation entries on tr2 are deleted losslessly (keep 3).
        let t2 = m.index().transform(2).unwrap();
        m.set_transform_rotation(t2, &[7.0, 0.0, 0.0]).unwrap();
        assert!(
            m.to_source().contains("*tr2 0 0 0 7 0 0\n"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn set_transform_fills_missing_displacement_component() {
        let mut m = Model::parse("t\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\ntr3 1\n");
        let t3 = m.index().transform(3).unwrap();
        m.set_transform_displacement(t3, [1.0, 2.0, 3.0]).unwrap();
        assert!(m.to_source().contains("tr3 1 2 3\n"), "{}", m.to_source());
    }

    #[test]
    fn slot_accessors_resolve_typed_views() {
        let m = Model::parse(MODEL);
        let surf_slots = m.surface_slots();
        assert_eq!(surf_slots.len(), 1);
        let s = m.surface_by_slot(surf_slots[0]).unwrap();
        assert_eq!(s.id, 1);
        // slot ↔ index round-trips.
        let ci = m.card_index_of_slot(surf_slots[0]).unwrap();
        assert_eq!(m.slot_at(ci), surf_slots[0]);
    }
}
