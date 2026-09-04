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
use std::fmt;

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

/// One dangling reference found by [`Model::validate`]. `cell`/`surface` is
/// `0` when the referencing card itself has no readable id (mirroring
/// [`crate::CellView::id`]'s own convention for a malformed card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    UndefinedSurface { cell: i64, surface: i64 },
    UndefinedMaterial { cell: i64, material: i64 },
    UndefinedCell { cell: i64, referenced: i64 },
    UndefinedUniverse { cell: i64, universe: i64 },
    UndefinedTransform { surface: i64, transform: i64 },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::UndefinedSurface { cell, surface } => {
                write!(f, "cell {cell}: references undefined surface {surface}")
            }
            Problem::UndefinedMaterial { cell, material } => {
                write!(f, "cell {cell}: references undefined material {material}")
            }
            Problem::UndefinedCell { cell, referenced } => {
                write!(f, "cell {cell}: references undefined cell {referenced}")
            }
            Problem::UndefinedUniverse { cell, universe } => {
                write!(
                    f,
                    "cell {cell}: fill references undefined universe {universe}"
                )
            }
            Problem::UndefinedTransform { surface, transform } => {
                write!(
                    f,
                    "surface {surface}: references undefined transform {transform}"
                )
            }
        }
    }
}

/// The kind of id family a [`Collision`] occurred in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IdKind {
    Cell,
    Surface,
    Material,
    Transform,
}

impl fmt::Display for IdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IdKind::Cell => "cell",
            IdKind::Surface => "surface",
            IdKind::Material => "material",
            IdKind::Transform => "transform",
        })
    }
}

/// A [`Model::merge`] failure: `id` is defined by more than one of the models
/// being combined.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Collision {
    pub kind: IdKind,
    pub id: i64,
    /// Index into `[self] ++ others` (so `0` is always `self`) of every model
    /// that defines `id`, ascending. The field a compositional tool actually
    /// needs to give a useful message — "these two fillers both define cell
    /// 100" — rather than just the id that collided.
    pub models: Vec<usize>,
}

impl fmt::Display for Collision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duplicate {} id {} (models ", self.kind, self.id)?;
        for (i, m) in self.models.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{m}")?;
        }
        f.write_str(")")
    }
}

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
    /// pointing at an undefined transform. Sorted and de-duplicated for a
    /// stable, diff-friendly result. An empty list means every reference
    /// resolves.
    pub fn validate(&self) -> Vec<Problem> {
        let universes: FxHashSet<i64> = self.cells().filter_map(|c| c.universe()).collect();

        let cells: Vec<CellView> = self.cells().collect();
        let mut out = validate_chunked(&cells, |c| {
            let mut local = Vec::new();
            let id = c.id().unwrap_or(0);
            for s in c.surface_ids() {
                if !self.surface_index.contains_key(&s) {
                    local.push(Problem::UndefinedSurface {
                        cell: id,
                        surface: s,
                    });
                }
            }
            if let Some(m) = c.material() {
                if m != 0 && !self.material_index.contains_key(&m) {
                    local.push(Problem::UndefinedMaterial {
                        cell: id,
                        material: m,
                    });
                }
            }
            for r in c.cell_refs() {
                if !self.cell_index.contains_key(&r) {
                    local.push(Problem::UndefinedCell {
                        cell: id,
                        referenced: r,
                    });
                }
            }
            if let Some(f) = c.fill() {
                // universe 0 is the (implicit) root; a fill of it is always valid.
                if f.universe != 0 && !universes.contains(&f.universe) {
                    local.push(Problem::UndefinedUniverse {
                        cell: id,
                        universe: f.universe,
                    });
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
                    local.push(Problem::UndefinedTransform {
                        surface: id,
                        transform: t,
                    });
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
    /// transforms that should not leak into a merge. A standalone `Comment`
    /// card sitting in the cell or surface block (e.g. component provenance
    /// notes) is kept too, in place; only comments that fall in the dropped
    /// data block are lost.
    ///
    /// Built by cloning the kept `Card`s directly, not by re-emitting to text
    /// and reparsing: a clone reuses each card's tokens as-is, so this pays no
    /// relex, only the (much cheaper) index scan every fresh `Model` needs. A
    /// caller that does this once per component of a multi-hundred-file project
    /// — its actual motivating use — used to pay a full lex of every kept
    /// card's text a second time; see `docs/05-parallelism-overhead.md`.
    pub fn clear_data_cards(&self) -> Model {
        let eol = self.cst.eol();
        let all: Vec<&Card> = self.cst.cards().collect();

        let mut cards: Vec<Card> = Vec::new();
        match all.iter().find(|c| c.kind() == CardKind::Title) {
            Some(title) => cards.push((*title).clone()),
            None => cards.push(Cst::new_card(
                CardKind::Title,
                format!("cleared{}", eol.as_str()),
            )),
        }

        // The cell and surface blocks are each a blank-line-free run bounded
        // by a Blank card, so everything up to the first Blank is the cell
        // block and everything between the first and second Blank is the
        // surface block — a standalone Comment card included, wherever it
        // falls in that run.
        let mut blanks = all
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind() == CardKind::Blank)
            .map(|(i, _)| i);
        let first_blank = blanks.next();
        let second_blank = blanks.next();

        let cell_end = first_blank.unwrap_or(all.len());
        cards.extend(
            all[..cell_end]
                .iter()
                .filter(|c| matches!(c.kind(), CardKind::Cell | CardKind::Comment))
                .map(|c| (*c).clone()),
        );
        cards.push(Cst::new_card(CardKind::Blank, eol.as_str().to_owned())); // delimiter: cells | surfaces

        if let Some(fb) = first_blank {
            let surf_end = second_blank.unwrap_or(all.len());
            cards.extend(
                all[fb + 1..surf_end]
                    .iter()
                    .filter(|c| matches!(c.kind(), CardKind::Surface | CardKind::Comment))
                    .map(|c| (*c).clone()),
            );
        }
        cards.push(Cst::new_card(CardKind::Blank, eol.as_str().to_owned())); // delimiter: surfaces | (empty) data

        Model::from_cst(Cst::from_cards(cards, eol))
    }

    /// In-place counterpart to [`Model::clear_data_cards`]: drops the data
    /// block (and its title/leading-`MESSAGE`-block normalisation) by
    /// draining rather than cloning. `clear_data_cards` materialises a full
    /// copy of every kept card into a fresh `Cst`; called once per component
    /// of a multi-hundred-file project — its motivating use — that copy of
    /// the (typically much larger) geometry is live at the same time as the
    /// original, which is real memory on a big build. This mutates `self`
    /// instead, so nothing is cloned; keep `clear_data_cards` for a caller
    /// that still needs the original afterwards.
    pub fn clear_data_cards_in_place(&mut self) {
        let mut past_title = false;
        let mut blanks_seen = 0u32;
        self.cst.retain(|c| match c.kind() {
            CardKind::Title => {
                past_title = true;
                true
            }
            CardKind::Blank if past_title && blanks_seen < 2 => {
                blanks_seen += 1;
                true
            }
            CardKind::Cell | CardKind::Surface | CardKind::Comment
                if past_title && blanks_seen < 2 =>
            {
                true
            }
            _ => false,
        });

        if !past_title {
            let text = format!("cleared{}", self.cst.eol().as_str());
            self.cst.insert_at(0, Cst::new_card(CardKind::Title, text));
        }
        // Always end with both delimiters, the same as `clear_data_cards`,
        // even if the source had no surfaces (or no cells) to begin with.
        while blanks_seen < 2 {
            let text = self.cst.eol().as_str().to_owned();
            self.cst.push(Cst::new_card(CardKind::Blank, text));
            blanks_seen += 1;
        }

        self.material_index.clear();
        self.transform_index.clear();
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

    /// Fold other models' cells, surfaces and data cards into this one. A
    /// standalone `Comment` card trailing one of those blocks travels with it.
    ///
    /// Fails (mutating nothing) if any cell / surface / material / transform id is
    /// shared across the models being combined — renumber the colliding family
    /// first, then merge. On success each incoming content card is appended to the
    /// end of its block and indexed, so the merged model is immediately queryable.
    ///
    /// `self` is model `0` in every [`Collision::models`]; `others[i]` is model
    /// `i + 1`.
    pub fn merge(&mut self, others: Vec<Model>) -> Result<(), Vec<Collision>> {
        let mut conflicts = Vec::new();
        {
            let all: Vec<&Model> = std::iter::once(&*self).chain(others.iter()).collect();
            conflicts.extend(shared_ids(&all, |m| &m.cell_index, IdKind::Cell));
            conflicts.extend(shared_ids(&all, |m| &m.surface_index, IdKind::Surface));
            conflicts.extend(shared_ids(&all, |m| &m.material_index, IdKind::Material));
            conflicts.extend(shared_ids(&all, |m| &m.transform_index, IdKind::Transform));
        }
        if !conflicts.is_empty() {
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
            // A standalone Comment card only ever trails the block it sits in
            // (segmentation absorbs any other comment into a card's header or
            // body), so it travels with whichever content kind was last seen —
            // landing in the same bucket, right after that block's cards, and
            // absorbed at the same position.
            let mut current_block: Option<CardKind> = None;
            for card in other.cst.into_cards() {
                match card.kind() {
                    CardKind::Cell | CardKind::Surface | CardKind::Data => {
                        current_block = Some(card.kind());
                        local.entry(card.kind()).or_default().push(card);
                    }
                    CardKind::Comment => {
                        if let Some(kind) = current_block {
                            local.entry(kind).or_default().push(card);
                        }
                    }
                    _ => {}
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
            match card.kind() {
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
fn validate_chunked<T: Sync, P: Send>(items: &[T], f: impl Fn(&T) -> Vec<P> + Sync) -> Vec<P> {
    if items.len() < PARALLEL_VALIDATE_THRESHOLD {
        return items.iter().flat_map(&f).collect();
    }
    let chunk = (items.len() / (rayon::current_num_threads().max(1) * 4)).max(1);
    items
        .par_chunks(chunk)
        .map(|c| c.iter().flat_map(&f).collect::<Vec<P>>())
        .collect::<Vec<Vec<P>>>()
        .into_iter()
        .flatten()
        .collect()
}

/// Ids that occur in more than one of `models` for one family — the collisions
/// that make a merge unsafe. `models[i]`'s own index into `models` is what
/// ends up on `Collision::models`, so the caller must pass `models` in the
/// same `[self] ++ others` order it documents on `Model::merge`.
///
/// Sorted by id: `owners` is an `FxHashMap`, so its iteration order is not
/// otherwise deterministic, and `merge` needs a stable result across runs.
fn shared_ids(
    models: &[&Model],
    index: impl Fn(&Model) -> &IdIndex,
    kind: IdKind,
) -> Vec<Collision> {
    let mut owners: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    for (mi, m) in models.iter().enumerate() {
        for &id in index(m).keys() {
            owners.entry(id).or_default().push(mi);
        }
    }
    let mut out: Vec<Collision> = owners
        .into_iter()
        .filter(|(_, models)| models.len() > 1)
        .map(|(id, models)| Collision { kind, id, models })
        .collect();
    out.sort_by_key(|c| c.id);
    out
}

#[cfg(test)]
mod tests {
    use super::{Collision, IdKind, Problem};
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
            problems.contains(&Problem::UndefinedSurface {
                cell: 1,
                surface: 9
            }),
            "{problems:?}"
        );
        assert!(
            problems.contains(&Problem::UndefinedMaterial {
                cell: 1,
                material: 3
            }),
            "{problems:?}"
        );
        assert!(
            problems.contains(&Problem::UndefinedTransform {
                surface: 1,
                transform: 4
            }),
            "{problems:?}"
        );
        // and Display renders the same text the old Vec<String> API did
        assert!(problems
            .iter()
            .any(|p| p.to_string() == "cell 1: references undefined surface 9"));
    }

    #[test]
    fn validate_is_clean_on_a_consistent_model() {
        let m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n");
        assert_eq!(m.validate(), Vec::<Problem>::new());
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
    fn clear_data_cards_keeps_trailing_block_comments() {
        let src = "t\n1 0 -1 imp:n=1\nc TRAILING CELL NOTE\n\n\
                   1 SO 10\nc TRAILING SURFACE NOTE\n\nM1 1001 1\n";
        let m = Model::parse(src);
        assert_eq!(m.to_source(), src);

        let cleared = m.clear_data_cards();
        assert!(cleared.to_source().contains("TRAILING CELL NOTE"));
        assert!(cleared.to_source().contains("TRAILING SURFACE NOTE"));
    }

    #[test]
    fn merged_component_comments_survive() {
        let filler =
            Model::parse("f\n100 0 -100 imp:n=1 u=1\nc FILLER NOTE\n\n100 SO 3\n\nM2 1001 1\n");
        let mut host =
            Model::parse("h\n1 0 -1 imp:n=1\n\n1 SO 10\n\nM1 1001 1\n").clear_data_cards();
        host.merge(vec![filler.clear_data_cards()]).unwrap();
        assert!(host.to_source().contains("FILLER NOTE"));
    }

    #[test]
    fn clear_data_cards_in_place_matches_the_cloning_version() {
        let cases = [
            "t\n1 1 -1.0 -1 imp:n=1\n2 0 -2 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n",
            "t\n1 0 -1 imp:n=1\nc TRAILING CELL NOTE\n\n1 SO 10\nc TRAILING SURFACE NOTE\n\nM1 1001 1\n",
            "t\n1 0 -1 imp:n=1\n\n\n", // no surfaces, no data at all
            "t\n\n\n",                 // no cells either
        ];
        for src in cases {
            let cloned = Model::parse(src).clear_data_cards();
            let mut in_place = Model::parse(src);
            in_place.clear_data_cards_in_place();
            assert_eq!(in_place.to_source(), cloned.to_source(), "input: {src:?}");
        }
    }

    #[test]
    fn clear_data_cards_in_place_drops_material_and_transform_indices() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n");
        assert!(m.material(1).is_some());
        assert!(m.transform(1).is_some());
        m.clear_data_cards_in_place();
        assert!(m.material(1).is_none());
        assert!(m.transform(1).is_none());
        assert!(m.cell(1).is_some(), "cell index must survive untouched");
    }

    #[test]
    fn clear_data_cards_in_place_synthesizes_a_title_when_absent() {
        let mut m = Model::parse("");
        m.clear_data_cards_in_place();
        assert_eq!(m.title(), Some("cleared"));
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
    fn extract_cells_keeps_the_cell_s_header_comment() {
        let m = Model::parse(
            "t\n\
             c fuel cell, see design note 7\n\
             1 1 -1.0 -1 imp:n=1\n\
             2 0 1 imp:n=0\n\
             \n\
             1 SO 5\n\
             \n\
             m1 1001 1\n",
        );
        assert!(m.cell(1).unwrap().text().contains("design note 7"));
        let extracted = m.extract_cells(&[1]);
        assert!(extracted.to_source().contains("design note 7"));
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
            err.contains(&Collision {
                kind: IdKind::Cell,
                id: 1,
                models: vec![0, 1],
            }),
            "{err:?}"
        );
        assert_eq!(err[0].to_string(), "duplicate cell id 1 (models 0, 1)");
        // nothing changed
        assert_eq!(a.to_source(), before);
        assert_eq!(a.num_cells(), 1);
    }
}
