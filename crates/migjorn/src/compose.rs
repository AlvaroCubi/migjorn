//! Analysis and composition (milestone M5): `validate`, `universe_ids`,
//! `extract_universe` / `extract_level0`, and `merge`.
//!
//! Extraction and merge are whole-model operations that produce or absorb cards
//! rather than edit one in place, so — unlike the edit/renumber paths — they are
//! allowed to go through parse: an extract assembles the chosen cards into source
//! text and re-parses it, which keeps the result trivially consistent. These are
//! not on the interactive hot path.

use migjorn_syntax::{Card, CardKind, Cst};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{IdIndex, Model};
use crate::view::{CellView, SurfaceView};
use crate::{cell, data, surface};

/// Below this many cells/surfaces, `validate` scans sequentially. `validate`
/// runs once on the fully-assembled model rather than nested inside a
/// caller's own parallel loop, so this threshold exists only to skip
/// dispatch overhead on models too small for it to pay off — not to avoid
/// nesting. See `docs/05-parallelism-overhead.md`.
const PARALLEL_VALIDATE_THRESHOLD: usize = 10_000;

/// Below this many incoming models, `merge`'s per-model card collection runs
/// sequentially — dispatch overhead isn't worth it for a handful of models.
const PARALLEL_MERGE_THRESHOLD: usize = 8;

impl Model {
    // --- analysis -----------------------------------------------------------

    /// Distinct universe ids declared by cells (`u=`), ascending. Level-0 cells
    /// (no `u=`) are not a numbered universe and do not appear here.
    pub fn universe_ids(&self) -> Vec<i64> {
        let mut u: Vec<i64> = self.cells().filter_map(|c| c.universe()).collect();
        u.sort_unstable();
        u.dedup();
        u
    }

    /// Cross-reference the model and report dangling references: a cell pointing
    /// at a surface / material / cell / universe that is not defined, or a surface
    /// pointing at an undefined transform. Messages are sorted and de-duplicated
    /// for a stable, diff-friendly result. An empty list means every reference
    /// resolves.
    pub fn validate(&self) -> Vec<String> {
        let universes: FxHashSet<i64> = self.cells().filter_map(|c| c.universe()).collect();

        let cells: Vec<CellView> = self.cells().collect();
        let mut out = validate_chunked(&cells, |c| {
            let mut local = Vec::new();
            let id = c.id().unwrap_or(0);
            for s in c.surface_ids() {
                if !self.surface_index.contains_key(&s) {
                    local.push(format!("cell {id}: references undefined surface {s}"));
                }
            }
            if let Some(m) = c.material() {
                if m != 0 && !self.material_index.contains_key(&m) {
                    local.push(format!("cell {id}: references undefined material {m}"));
                }
            }
            for r in c.cell_refs() {
                if !self.cell_index.contains_key(&r) {
                    local.push(format!("cell {id}: references undefined cell {r}"));
                }
            }
            if let Some(f) = c.fill() {
                // universe 0 is the (implicit) root; a fill of it is always valid.
                if f.universe != 0 && !universes.contains(&f.universe) {
                    local.push(format!(
                        "cell {id}: fill references undefined universe {}",
                        f.universe
                    ));
                }
            }
            local
        });

        let surfaces: Vec<SurfaceView> = self.surfaces().collect();
        out.extend(validate_chunked(&surfaces, |s| {
            let mut local = Vec::new();
            if let Some(t) = s.transform() {
                let t = t.abs();
                if !self.transform_index.contains_key(&t) {
                    let id = s.id().unwrap_or(0);
                    local.push(format!("surface {id}: references undefined transform {t}"));
                }
            }
            local
        }));

        out.sort();
        out.dedup();
        out
    }

    // --- extraction ---------------------------------------------------------

    /// A standalone model containing just universe `u`'s cells and the surfaces,
    /// materials and transforms they use.
    ///
    /// It does **not** recurse into universes those cells `fill=`, it is the one
    /// universe's own cards, so a `fill=` to a sub-universe will dangle (which
    /// [`Model::validate`] reports). Pull sub-universes with further calls, or
    /// use [`Model::extract_cells`] for a recursive walk.
    pub fn extract_universe(&self, u: i64) -> Model {
        let keep: FxHashSet<i64> = self
            .cells()
            .filter(|c| c.universe() == Some(u))
            .filter_map(|c| c.id())
            .collect();
        self.extract(&keep)
    }

    /// A standalone model of the level-0 (root) cells — those with no `u=` — and
    /// the surfaces, materials and transforms they use.
    ///
    /// Like [`Model::extract_universe`], this does **not** recurse into filled
    /// sub-universes; use [`Model::extract_cells`] for that.
    pub fn extract_level0(&self) -> Model {
        let keep: FxHashSet<i64> = self
            .cells()
            .filter(|c| c.universe().is_none())
            .filter_map(|c| c.id())
            .collect();
        self.extract(&keep)
    }

    /// A standalone model of the given cells and everything they need to be
    /// self-contained: their surfaces, materials and transforms, plus —
    /// recursively — any cell reached through a `fill=` (which pulls in that
    /// universe's own cells), a `LIKE n BUT` base, or a `#n` complement. Ids
    /// that don't name an existing cell are silently ignored.
    pub fn extract_cells(&self, ids: &[i64]) -> Model {
        // Cells grouped by declared universe, so a `fill=` pull is a lookup
        // rather than a rescan of every cell.
        let mut by_universe: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
        for c in self.cells() {
            if let (Some(u), Some(id)) = (c.universe(), c.id()) {
                by_universe.entry(u).or_default().push(id);
            }
        }

        let mut keep: FxHashSet<i64> = FxHashSet::default();
        let mut stack: Vec<i64> = ids.to_vec();
        while let Some(id) = stack.pop() {
            if !keep.insert(id) {
                continue; // already visited: fill/like cycles terminate here
            }
            let Some(c) = self.cell(id) else { continue };
            stack.extend(c.cell_refs());
            if let Some(f) = c.fill() {
                if let Some(children) = by_universe.get(&f.universe) {
                    stack.extend(children.iter().copied());
                }
            }
        }

        self.extract(&keep)
    }

    /// A copy of this model with its title, cells and surfaces kept and its data
    /// block dropped — e.g. before folding in a component's own materials/
    /// transforms that should not leak into a merge.
    ///
    /// Built by cloning the kept `Card`s directly, not by re-emitting to text
    /// and reparsing: a clone reuses each card's tokens as-is, so this pays no
    /// relex, only the (much cheaper) index scan every fresh `Model` needs. A
    /// caller that does this once per component of a multi-hundred-file project
    /// — its actual motivating use — used to pay a full lex of every kept
    /// card's text a second time; see `docs/05-parallelism-overhead.md`.
    pub fn clear_data_cards(&self) -> Model {
        let eol = self.cst.eol();

        let mut cards: Vec<Card> = Vec::new();
        match self.cst.cards().find(|c| c.kind() == CardKind::Title) {
            Some(title) => cards.push(title.clone()),
            None => cards.push(Cst::new_card(
                CardKind::Title,
                format!("cleared{}", eol.as_str()),
            )),
        }
        cards.extend(
            self.cst
                .cards()
                .filter(|c| c.kind() == CardKind::Cell)
                .cloned(),
        );
        cards.push(Cst::new_card(CardKind::Blank, eol.as_str().to_owned())); // delimiter: cells | surfaces
        cards.extend(
            self.cst
                .cards()
                .filter(|c| c.kind() == CardKind::Surface)
                .cloned(),
        );
        cards.push(Cst::new_card(CardKind::Blank, eol.as_str().to_owned())); // delimiter: surfaces | (empty) data

        Model::from_cst(Cst::from_cards(cards, eol))
    }

    fn extract(&self, keep: &FxHashSet<i64>) -> Model {
        let eol = self.cst.eol().as_str();

        let mut cell_src = String::new();
        let mut surf_ids: FxHashSet<i64> = FxHashSet::default();
        let mut mat_ids: FxHashSet<i64> = FxHashSet::default();
        for c in self.cells() {
            if c.id().is_some_and(|id| keep.contains(&id)) {
                cell_src.push_str(c.text());
                surf_ids.extend(c.surface_ids());
                if let Some(m) = c.material() {
                    if m != 0 {
                        mat_ids.insert(m);
                    }
                }
            }
        }

        let mut surf_src = String::new();
        let mut tr_ids: FxHashSet<i64> = FxHashSet::default();
        for s in self.surfaces() {
            if s.id().is_some_and(|id| surf_ids.contains(&id)) {
                surf_src.push_str(s.text());
                if let Some(t) = s.transform() {
                    tr_ids.insert(t.abs());
                }
            }
        }

        // Data cards in file order, keeping only the referenced materials and
        // transforms.
        let mut data_src = String::new();
        for card in self.cst.cards() {
            if card.kind() != CardKind::Data {
                continue;
            }
            if let Some(h) = data::head(card) {
                let referenced = data::material_id(&h).is_some_and(|id| mat_ids.contains(&id))
                    || data::transform_id(&h).is_some_and(|id| tr_ids.contains(&id));
                if referenced {
                    data_src.push_str(card.text());
                }
            }
        }

        let title = self
            .cst
            .cards()
            .find(|c| c.kind() == CardKind::Title)
            .map(|c| c.text().to_owned())
            .unwrap_or_else(|| format!("extracted{eol}"));

        let mut src = String::with_capacity(
            title.len() + cell_src.len() + surf_src.len() + data_src.len() + 2 * eol.len(),
        );
        src.push_str(&title);
        src.push_str(&cell_src);
        src.push_str(eol); // blank delimiter: cells | surfaces
        src.push_str(&surf_src);
        src.push_str(eol); // blank delimiter: surfaces | data
        src.push_str(&data_src);
        Model::parse(&src)
    }

    // --- merge --------------------------------------------------------------

    /// Fold other models' cells, surfaces and data cards into this one.
    ///
    /// Fails (mutating nothing) if any cell / surface / material / transform id is
    /// shared across the models being combined — renumber the colliding family
    /// first, then merge. On success each incoming content card is appended to the
    /// end of its block and indexed, so the merged model is immediately queryable.
    pub fn merge(&mut self, others: Vec<Model>) -> Result<(), Vec<String>> {
        let mut conflicts = Vec::new();
        {
            let all: Vec<&Model> = std::iter::once(&*self).chain(others.iter()).collect();
            conflicts.extend(shared_ids(&all, |m| &m.cell_index, "cell"));
            conflicts.extend(shared_ids(&all, |m| &m.surface_index, "surface"));
            conflicts.extend(shared_ids(&all, |m| &m.material_index, "material"));
            conflicts.extend(shared_ids(&all, |m| &m.transform_index, "transform"));
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            return Err(conflicts);
        }

        // Group incoming cards by kind (preserving relative order) so each block
        // is absorbed with a single batched insert rather than one O(n) insert
        // per card — the latter turns a whole-project merge quadratic in the
        // combined model size.
        //
        // Collecting each incoming model's own cards is embarrassingly parallel
        // across models (unlike absorbing the result, which mutates `self` and
        // must stay sequential and ordered); below the threshold this collects
        // one model at a time exactly as before. Each `other` is consumed and
        // its cards moved out via `into_cards`, not re-lexed from cloned text:
        // every card here was already lexed once, by whatever `Model::parse`
        // or `clear_data_cards` produced it, so absorbing it should cost an
        // insert, not a second lex pass over millions of cards.
        let mut by_kind: FxHashMap<CardKind, Vec<Card>> = FxHashMap::default();
        let collect_one = |other: Model| -> FxHashMap<CardKind, Vec<Card>> {
            let mut local: FxHashMap<CardKind, Vec<Card>> = FxHashMap::default();
            for card in other.cst.into_cards() {
                if matches!(
                    card.kind(),
                    CardKind::Cell | CardKind::Surface | CardKind::Data
                ) {
                    local.entry(card.kind()).or_default().push(card);
                }
            }
            local
        };
        let per_model: Vec<FxHashMap<CardKind, Vec<Card>>> =
            if others.len() < PARALLEL_MERGE_THRESHOLD {
                others.into_iter().map(collect_one).collect()
            } else {
                others.into_par_iter().map(collect_one).collect()
            };
        for mut local in per_model {
            for kind in [CardKind::Cell, CardKind::Surface, CardKind::Data] {
                if let Some(mut cards) = local.remove(&kind) {
                    by_kind.entry(kind).or_default().append(&mut cards);
                }
            }
        }
        for kind in [CardKind::Cell, CardKind::Surface, CardKind::Data] {
            if let Some(cards) = by_kind.remove(&kind) {
                self.absorb_many(kind, cards);
            }
        }
        Ok(())
    }

    /// Append a batch of same-kind content cards at the end of that block, in
    /// one structural edit, and index each one's id. `cards` are already-lexed
    /// (moved out of another `Cst` by the caller, e.g. via [`Cst::into_cards`]),
    /// not built from text here — absorbing must not re-lex.
    fn absorb_many(&mut self, kind: CardKind, cards: Vec<Card>) {
        let at = self
            .cst
            .end_of_block(kind)
            .unwrap_or_else(|| self.cst.len());
        let slots = self.cst.insert_many_at(at, cards);
        for slot in slots {
            let card = self.cst.card(slot).unwrap();
            match kind {
                CardKind::Cell => {
                    if let Some(id) = cell::layout(card).id {
                        self.cell_index.entry(id).or_insert(slot);
                    }
                }
                CardKind::Surface => {
                    if let Some(id) = surface::layout(card).id {
                        self.surface_index.entry(id).or_insert(slot);
                    }
                }
                CardKind::Data => {
                    if let Some(h) = data::head(card) {
                        if let Some(id) = data::material_id(&h) {
                            self.material_index.entry(id).or_insert(slot);
                        } else if let Some(id) = data::transform_id(&h) {
                            self.transform_index.entry(id).or_insert(slot);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Map `f` over `items` and flatten the results, in chunks large enough to be
/// worth a rayon dispatch; below `PARALLEL_VALIDATE_THRESHOLD` this is a plain
/// sequential scan. Same chunk-then-flatten shape as `Model::build_indices`.
fn validate_chunked<T: Sync>(items: &[T], f: impl Fn(&T) -> Vec<String> + Sync) -> Vec<String> {
    if items.len() < PARALLEL_VALIDATE_THRESHOLD {
        return items.iter().flat_map(&f).collect();
    }
    let chunk = (items.len() / (rayon::current_num_threads().max(1) * 4)).max(1);
    items
        .par_chunks(chunk)
        .map(|c| c.iter().flat_map(&f).collect::<Vec<String>>())
        .collect::<Vec<Vec<String>>>()
        .concat()
}

/// Ids that occur in more than one of `models` for one family — the collisions
/// that make a merge unsafe.
fn shared_ids(models: &[&Model], index: impl Fn(&Model) -> &IdIndex, label: &str) -> Vec<String> {
    let mut owners: FxHashMap<i64, usize> = FxHashMap::default();
    for m in models {
        for &id in index(m).keys() {
            *owners.entry(id).or_insert(0) += 1;
        }
    }
    owners
        .into_iter()
        .filter(|&(_, count)| count > 1)
        .map(|(id, _)| format!("duplicate {label} id {id}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Model;

    #[test]
    fn universe_ids_are_distinct_and_sorted() {
        let m = Model::parse(
            "t\n1 0 -1 u=5 imp:n=1\n2 0 -2 u=2 imp:n=1\n3 0 -3 u=5 imp:n=1\n4 0 -4 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n4 SO 8\n\nm1 1001 1\n",
        );
        assert_eq!(m.universe_ids(), vec![2, 5]);
    }

    #[test]
    fn validate_flags_dangling_references() {
        // cell 1 references surface 9 (undefined) and material 3 (undefined);
        // surface 1 references transform 4 (undefined).
        let m = Model::parse("t\n1 3 -1.0 -9 imp:n=1\n\n1 4 SO 5\n\nm1 1001 1\n");
        let problems = m.validate();
        assert!(
            problems.iter().any(|p| p.contains("undefined surface 9")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("undefined material 3")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("undefined transform 4")),
            "{problems:?}"
        );
    }

    #[test]
    fn validate_is_clean_on_a_consistent_model() {
        let m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n");
        assert_eq!(m.validate(), Vec::<String>::new());
    }

    #[test]
    fn extract_universe_pulls_cells_and_their_dependencies() {
        let m = Model::parse(
            "t\n\
             1 1 -1.0 -1 u=5 imp:n=1\n\
             2 0 -2 u=7 imp:n=1\n\
             \n\
             1 SO 5\n\
             2 SO 6\n\
             \n\
             m1 1001 1\nm2 8016 1\n",
        );
        let u5 = m.extract_universe(5);
        // universe 5's cell, its surface, and its material only
        assert_eq!(u5.num_cells(), 1);
        assert!(u5.cell(1).is_some());
        assert!(u5.surface(1).is_some());
        assert!(u5.surface(2).is_none());
        assert!(u5.material(1).is_some());
        assert!(u5.material(2).is_none());
    }

    #[test]
    fn clear_data_cards_keeps_title_cells_surfaces_and_drops_data() {
        let m =
            Model::parse("t\n1 1 -1.0 -1 imp:n=1\n2 0 -2 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n");
        let cleared = m.clear_data_cards();
        assert_eq!(cleared.num_cells(), 2);
        assert!(cleared.cell(1).is_some());
        assert!(cleared.surface(1).is_some());
        assert!(cleared.surface(2).is_some());
        assert!(cleared.material(1).is_none());
        let out = cleared.to_source();
        assert!(out.starts_with("t\n"));
        assert!(!out.contains("m1 1001 1"));

        // composes cleanly with a later-appended data block: the reduced model
        // still ends with the surfaces/data blank delimiter, so merging in a
        // model that carries only data cards lands them after the surfaces
        // rather than swallowing them into the surface block.
        let data_only = Model::parse("data only\n\n\nm1 1001 1\n");
        let mut cleared = cleared;
        cleared.merge(vec![data_only]).unwrap();
        assert!(cleared.material(1).is_some());
        assert!(cleared.to_source().contains("2 SO 6\n\nm1 1001 1\n"));
    }

    #[test]
    fn extract_level0_takes_only_rootless_cells() {
        let m =
            Model::parse("t\n1 0 -1 imp:n=1\n2 0 -2 u=5 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n");
        let root = m.extract_level0();
        assert_eq!(root.num_cells(), 1);
        assert!(root.cell(1).is_some());
        assert!(root.cell(2).is_none());
        assert!(root.surface(1).is_some());
        assert!(root.surface(2).is_none());
    }

    #[test]
    fn extract_universe_does_not_recurse_into_filled_sub_universes() {
        // cell 1 (universe 5) fills in universe 7; extract_universe(5) must
        // pull only universe 5's own cell and leave the fill= dangling, per
        // its documented contract. Callers that want the closure use
        // extract_cells.
        let m = Model::parse(
            "t\n\
             1 0 -1 u=5 fill=7 imp:n=1\n\
             2 0 -2 u=7 imp:n=1\n\
             \n\
             1 SO 5\n2 SO 6\n\
             \nm1 1001 1\n",
        );
        let u5 = m.extract_universe(5);
        assert_eq!(u5.num_cells(), 1);
        assert!(u5.cell(1).is_some());
        assert!(u5.cell(2).is_none());
        assert!(u5.surface(2).is_none());
    }

    #[test]
    fn extract_level0_does_not_recurse_into_filled_sub_universes() {
        let m = Model::parse(
            "t\n\
             1 0 -1 fill=5 imp:n=1\n\
             2 0 -2 u=5 imp:n=1\n\
             \n\
             1 SO 5\n2 SO 6\n\
             \nm1 1001 1\n",
        );
        let root = m.extract_level0();
        assert_eq!(root.num_cells(), 1);
        assert!(root.cell(1).is_some());
        assert!(root.cell(2).is_none());
        assert!(root.surface(2).is_none());
    }

    #[test]
    fn extract_cells_recurses_into_fill_like_and_complement_refs() {
        let m = Model::parse(
            "t\n\
             1 0 -1 fill=5 imp:n=1\n\
             2 1 -1.0 -2 u=5 imp:n=1\n\
             3 0 -3 #2 imp:n=1\n\
             4 LIKE 3 BUT trcl=1 imp:n=1\n\
             5 0 -4 imp:n=1\n\
             \n\
             1 SO 5\n2 SO 6\n3 SO 7\n4 SO 8\n\
             \nm1 1001 1\n",
        );
        let extracted = m.extract_cells(&[4]);
        // 4 -> LIKE 3, 3 -> #2 complement, 2 has no further refs; 1 and 5 are
        // untouched.
        assert_eq!(extracted.num_cells(), 3);
        assert!(extracted.cell(4).is_some());
        assert!(extracted.cell(3).is_some());
        assert!(extracted.cell(2).is_some());
        assert!(extracted.cell(1).is_none());
        assert!(extracted.cell(5).is_none());
    }

    #[test]
    fn extract_cells_ignores_unknown_ids() {
        let m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let extracted = m.extract_cells(&[1, 999]);
        assert_eq!(extracted.num_cells(), 1);
    }

    #[test]
    fn merge_combines_disjoint_models() {
        let mut a = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let b = Model::parse("t\n2 2 -2.0 -2 imp:n=1\n\n2 SO 6\n\nm2 8016 1\n");
        assert!(a.merge(vec![b]).is_ok());
        assert_eq!(a.num_cells(), 2);
        assert!(a.cell(2).is_some());
        assert!(a.surface(2).is_some());
        assert!(a.material(2).is_some());
        let out = a.to_source();
        assert!(out.contains("1 SO 5"));
        assert!(out.contains("2 SO 6"));
    }

    #[test]
    fn merge_rejects_id_collisions_without_mutating() {
        let mut a = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let before = a.to_source();
        let b = Model::parse("t\n1 2 -2.0 -2 imp:n=1\n\n2 SO 6\n\nm2 8016 1\n");
        let err = a.merge(vec![b]).unwrap_err();
        assert!(
            err.iter().any(|e| e.contains("duplicate cell id 1")),
            "{err:?}"
        );
        // nothing changed
        assert_eq!(a.to_source(), before);
        assert_eq!(a.num_cells(), 1);
    }
}
