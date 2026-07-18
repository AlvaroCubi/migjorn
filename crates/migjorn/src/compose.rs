//! Model *assembly* workflows: splitting a model into universe sub-models and
//! re-linking them.
//!
//! These read a cell's `u=`/`fill=`, carve a universe (or the level-0 shell)
//! into a standalone [`Model`], drop a source model's data block, splice an
//! inline comment, and merge disjoint models with collision detection. The
//! extraction follows *every* reference a kept cell makes — surfaces, `#n` cell
//! complements and `LIKE n` bases (transitively), materials, and transforms —
//! and carries only the data cards those references need, so the result runs on
//! its own.

use migjorn_syntax::{GreenTree, SyntaxKind};
use rustc_hash::FxHashSet;

use crate::cell::{
    cell_id, cell_material, cell_trcl_transform, cell_universe, cells, parse_fill, scan_cell_refs,
    Fill, RefKind,
};
use crate::datacard::parse_data_card;
use crate::material::{materials, parse_material};
use crate::model::{EditError, Model, ModelView};
use crate::surface::{parse_surface, surfaces};
use crate::transform::{parse_transform, transforms};

impl ModelView<'_> {
    /// The universe of the cell at `card_index` (its `u=` value), or `None` if
    /// the cell has no `u=` parameter (a level-0 / real-world cell).
    pub fn cell_universe(&self, card_index: usize) -> Option<i64> {
        cell_universe(self.tree, card_index)
    }

    /// The `fill=`/`*fill=` of the cell at `card_index` (simple single-universe
    /// form), or `None` if the cell is not filled.
    pub fn cell_fill(&self, card_index: usize) -> Option<Fill> {
        parse_fill(self.tree, card_index)
    }

    /// Every universe defined by a `u=` in the model, sorted ascending and
    /// deduplicated. Universe 0 (the real world) is not reported.
    pub fn universe_ids(&self) -> Vec<i64> {
        let mut set: FxHashSet<i64> = FxHashSet::default();
        for i in 0..self.tree.cards().len() {
            if let Some(u) = cell_universe(self.tree, i) {
                if u != 0 {
                    set.insert(u);
                }
            }
        }
        let mut ids: Vec<i64> = set.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// Carve universe `u` into a standalone [`Model`], as
    /// [`Model::extract_universe`]. A view only ever exists over an already-
    /// materialized tree, so this takes `&self` and composes like every other
    /// view reader: call it from multiple threads (e.g. via `rayon`'s
    /// `par_iter`) to extract every universe in parallel, since `ModelView`
    /// borrows the tree immutably and is `Sync`.
    pub fn extract_universe(&self, u: i64) -> Model {
        extract_cells(self.tree, |cu| cu == Some(u))
    }

    /// Carve the level-0 shell into a standalone [`Model`], as
    /// [`Model::extract_level0`]. See [`ModelView::extract_universe`] for why
    /// this is `&self`.
    pub fn extract_level0(&self) -> Model {
        extract_cells(self.tree, |cu| cu.is_none())
    }
}

impl Model {
    /// Carve universe `u` into a standalone [`Model`]: the cells whose `u=` is
    /// `u`, plus everything they reference — surfaces, the cells reached through
    /// `#n` complements and `LIKE n` bases (followed transitively), and the
    /// materials and transforms those cells and surfaces use. Only the data
    /// cards those references need are carried (referenced `Mn`/`TRn` and their
    /// companions), while global cards such as the source (`SDEF`, `KCODE`, …)
    /// pass through, so the extraction is runnable on its own — call
    /// [`Model::clear_data_cards`] for a geometry-only sub-model instead.
    ///
    /// The extracted cells keep their `u=`, so the result is a valid filler
    /// universe. Following `#n`/`LIKE` references can pull in cells from other
    /// universes when the geometry crosses universe boundaries; for a model whose
    /// universes are self-contained this is exactly the universe's cells.
    pub fn extract_universe(&mut self, u: i64) -> Model {
        self.materialize();
        extract_cells(&self.tree, |cu| cu == Some(u))
    }

    /// Carve the level-0 shell — every cell with no `u=` — plus everything those
    /// cells reference into a standalone [`Model`], following the same rules as
    /// [`Model::extract_universe`]. For a model whose universes are self-contained
    /// this is the inverse selection of `extract_universe` and the two partition
    /// the model's cells.
    pub fn extract_level0(&mut self) -> Model {
        self.materialize();
        extract_cells(&self.tree, |cu| cu.is_none())
    }

    /// Drop every data card, keeping the title, cell block, and surface block.
    /// The model is re-parsed from the reduced source (stable slots are not
    /// preserved). Useful when composing a model whose data cards come from
    /// elsewhere and a source model's own data block should be ignored.
    pub fn clear_data_cards(&mut self) {
        let title = self.title_source();
        let cells = self.card_texts(SyntaxKind::CELL_CARD);
        let surfaces = self.card_texts(SyntaxKind::SURFACE_CARD);
        *self = Model::parse(assemble_blocks(&title, &cells, &surfaces, &[]));
    }

    /// Append ` $ text` as an inline comment to the card at `card_index`, after
    /// its last content token. Lossless: every other byte of the card is kept.
    /// Applies to any card kind; `EditError::NotACell` here means the index is
    /// out of range or the card has no content to anchor the comment to.
    pub fn append_inline_comment(
        &mut self,
        card_index: usize,
        text: &str,
    ) -> Result<(), EditError> {
        let card = *self
            .tree
            .cards()
            .get(card_index)
            .ok_or(EditError::NotACell)?;
        let last = self
            .tree
            .card_content_tokens(&card)
            .last()
            .ok_or(EditError::NotACell)?;
        self.tree.insert_after(last, format!(" $ {text}"));
        Ok(())
    }

    /// Merge `others` into `self`, appending their cells, surfaces, and data
    /// cards to the respective blocks. Honors the disjoint-range convention:
    /// if any cell, surface, material, or transform id is defined by more than
    /// one of the merged models, **nothing is changed** and every conflict is
    /// returned. On success the model is re-parsed from the combined source
    /// (stable slots are not preserved across a merge).
    pub fn merge(&mut self, others: &[&Model]) -> Result<(), Vec<MergeConflict>> {
        let conflicts = self.merge_conflicts(others);
        if !conflicts.is_empty() {
            return Err(conflicts);
        }
        let title = self.title_source();
        let mut cells = self.card_texts(SyntaxKind::CELL_CARD);
        let mut surfaces = self.card_texts(SyntaxKind::SURFACE_CARD);
        let mut data = self.card_texts(SyntaxKind::DATA_CARD);
        for o in others {
            cells.extend(o.card_texts(SyntaxKind::CELL_CARD));
            surfaces.extend(o.card_texts(SyntaxKind::SURFACE_CARD));
            data.extend(o.card_texts(SyntaxKind::DATA_CARD));
        }
        *self = Model::parse(assemble_blocks(&title, &cells, &surfaces, &data));
        Ok(())
    }

    /// Every id defined by more than one of `[self, others…]`, per kind, in a
    /// deterministic order (self before others, cells, surfaces, materials, then
    /// transforms).
    fn merge_conflicts(&self, others: &[&Model]) -> Vec<MergeConflict> {
        let mut conflicts = Vec::new();
        let mut cells_seen: FxHashSet<i64> = FxHashSet::default();
        let mut surfaces_seen: FxHashSet<i64> = FxHashSet::default();
        let mut materials_seen: FxHashSet<i64> = FxHashSet::default();
        let mut transforms_seen: FxHashSet<i64> = FxHashSet::default();
        // Read via the free projections rather than `Model::view`: `others` are
        // shared refs, and a view would need `&mut` on models we only read.
        // Safe without materializing — ids are override-only, so `token_text`
        // already applies any pending edit, and the `card_source`-based merge
        // emits each `other`'s effective text regardless.
        for m in std::iter::once(self).chain(others.iter().copied()) {
            for c in cells(&m.tree) {
                if !cells_seen.insert(c.id) {
                    conflicts.push(MergeConflict::new(ConflictKind::Cell, c.id));
                }
            }
            for s in surfaces(&m.tree) {
                if !surfaces_seen.insert(s.id) {
                    conflicts.push(MergeConflict::new(ConflictKind::Surface, s.id));
                }
            }
            for mat in materials(&m.tree) {
                if !materials_seen.insert(mat.id) {
                    conflicts.push(MergeConflict::new(ConflictKind::Material, mat.id));
                }
            }
            for t in transforms(&m.tree) {
                if !transforms_seen.insert(t.id) {
                    conflicts.push(MergeConflict::new(ConflictKind::Transform, t.id));
                }
            }
        }
        conflicts
    }

    /// Source text of every card of `kind`, in order (each trimmed of trailing
    /// whitespace).
    fn card_texts(&self, kind: SyntaxKind) -> Vec<String> {
        (0..self.tree.cards().len())
            .filter(|&i| self.tree.cards()[i].kind == kind)
            .map(|i| self.tree.card_source(i).trim_end().to_string())
            .collect()
    }

    /// Source text of the model's title card, or `"title"` if it has none.
    fn title_source(&self) -> String {
        (0..self.tree.cards().len())
            .find(|&i| self.tree.cards()[i].kind == SyntaxKind::TITLE_CARD)
            .map(|i| self.tree.card_source(i).trim_end().to_string())
            .unwrap_or_else(|| "title".to_string())
    }
}

/// Shared engine for the `extract_*` methods: seed the kept set with the
/// cells whose universe satisfies `keep`, follow every reference to a closed
/// set of cells/surfaces/materials/transforms, carry only the needed data
/// cards, and re-parse a title + cell + surface + data model. Takes `&GreenTree`
/// rather than `&Model` so it is callable from both `Model` (after the `&mut`
/// materialize) and `ModelView` (already guaranteed materialized) — the latter
/// is what lets callers extract several universes in parallel.
fn extract_cells(tree: &GreenTree, keep: impl Fn(Option<i64>) -> bool) -> Model {
    let ncards = tree.cards().len();

    // Cell id → card index, so `#n`/`LIKE n` references can be followed.
    let mut cell_card: rustc_hash::FxHashMap<i64, usize> = rustc_hash::FxHashMap::default();
    for i in 0..ncards {
        if let Some((_, id)) = cell_id(tree, i) {
            cell_card.entry(id).or_insert(i);
        }
    }

    // Seed the worklist with the cells whose universe satisfies `keep`.
    let mut kept: FxHashSet<usize> = FxHashSet::default();
    let mut work: Vec<usize> = Vec::new();
    for i in 0..ncards {
        if tree.cards()[i].kind == SyntaxKind::CELL_CARD
            && keep(cell_universe(tree, i))
            && kept.insert(i)
        {
            work.push(i);
        }
    }

    // Follow every reference to a closed set of cells and gather the
    // surfaces, materials, and transforms the kept cells use.
    let mut wanted_surfaces: FxHashSet<i64> = FxHashSet::default();
    let mut wanted_materials: FxHashSet<i64> = FxHashSet::default();
    let mut wanted_transforms: FxHashSet<i64> = FxHashSet::default();
    while let Some(i) = work.pop() {
        if let Some((_, mat)) = cell_material(tree, i) {
            if mat != 0 {
                wanted_materials.insert(mat);
            }
        }
        // A `fill=u (TRn)` or `trcl=TRn` bare reference needs that transform.
        if let Some(t) = parse_fill(tree, i)
            .and_then(|f| f.transform)
            .and_then(|s| s.parse::<i64>().ok())
        {
            wanted_transforms.insert(t.abs());
        }
        if let Some(t) = cell_trcl_transform(tree, i) {
            wanted_transforms.insert(t);
        }
        scan_cell_refs(tree, i, |kind, _tok, val| match kind {
            RefKind::SurfaceRef { .. } => {
                wanted_surfaces.insert(val);
            }
            RefKind::CellRef => {
                if let Some(&ci) = cell_card.get(&val) {
                    if kept.insert(ci) {
                        work.push(ci);
                    }
                }
            }
            RefKind::CellId => {}
        });
    }

    // Walk every card once, in source order, splitting each into its own
    // text and any trailing comment block it absorbed (`card_header_split`).
    // That trailing block is a header an author wrote for whatever card
    // comes *next* — the card-boundary rule just misattaches it backward
    // onto this one (see `build_cards`). A header trailing a dropped card
    // therefore carries forward as `pending_header` onto whichever kept
    // card follows, instead of being discarded with its host.
    let mut title_text = "title".to_string();
    let mut cell_texts: Vec<String> = Vec::new();
    let mut surface_texts: Vec<String> = Vec::new();
    let mut data_texts: Vec<String> = Vec::new();
    let mut pending_header: Option<String> = None;
    for i in 0..ncards {
        let kind = tree.cards()[i].kind;
        let (own, header) = card_text_and_header(tree, i);

        if kind == SyntaxKind::TITLE_CARD {
            title_text = own;
            pending_header = header;
            continue;
        }

        let is_kept = match kind {
            SyntaxKind::CELL_CARD => kept.contains(&i),
            SyntaxKind::SURFACE_CARD => match parse_surface(tree, i) {
                Some(s) if wanted_surfaces.contains(&s.id) => {
                    if let Some(t) = s.transform {
                        wanted_transforms.insert(t.abs());
                    }
                    true
                }
                _ => false,
            },
            SyntaxKind::DATA_CARD => data_card_needed(tree, i, &wanted_materials, &wanted_transforms),
            _ => false,
        };

        if is_kept {
            let text = match pending_header.take() {
                Some(h) => format!("{h}\n{own}"),
                None => own,
            };
            match kind {
                SyntaxKind::CELL_CARD => cell_texts.push(text),
                SyntaxKind::SURFACE_CARD => surface_texts.push(text),
                SyntaxKind::DATA_CARD => data_texts.push(text),
                _ => unreachable!("filtered to cell/surface/data above"),
            }
            pending_header = header;
        } else if header.is_some() {
            pending_header = header;
        }
    }

    Model::parse(assemble_blocks(
        &title_text,
        &cell_texts,
        &surface_texts,
        &data_texts,
    ))
}

/// A card's own source text (through its last content line, trimmed of
/// trailing whitespace) and, if it absorbed a trailing block of whole `c`/`$`
/// comment lines, that header (trimmed) — the header an author wrote to
/// introduce whichever card follows, misattached backward by the
/// card-boundary rule. See `GreenTree::card_header_split`.
fn card_text_and_header(tree: &GreenTree, card_index: usize) -> (String, Option<String>) {
    let card = tree.cards()[card_index];
    let split = tree.card_header_split(card_index);
    let own = tree
        .token_range_source(card.first_tok, split)
        .trim_end()
        .to_string();
    let header = tree.token_range_source(split, card.tok_end);
    let header = header.trim();
    (own, (!header.is_empty()).then(|| header.to_string()))
}

/// Whether the data card at `card_index` must be carried into an extraction:
/// a referenced `Mn` material (with its `MTn` thermal companion) or a referenced
/// `TRn` transform. Every other data card (source, physics, mode, tallies, …) is
/// global to the model and is always carried.
fn data_card_needed(
    tree: &GreenTree,
    card_index: usize,
    materials: &FxHashSet<i64>,
    transforms: &FxHashSet<i64>,
) -> bool {
    if let Some(m) = parse_material(tree, card_index) {
        return materials.contains(&m.id);
    }
    if let Some(t) = parse_transform(tree, card_index) {
        return transforms.contains(&t.id);
    }
    if let Some(base) = mt_material_id(tree, card_index) {
        return materials.contains(&base);
    }
    true
}

/// The material number of an `MTn` thermal-scattering card at `card_index`, or
/// `None` if the card is not an `MTn`.
fn mt_material_id(tree: &GreenTree, card_index: usize) -> Option<i64> {
    let dc = parse_data_card(tree, card_index)?;
    let digits = dc.name.strip_prefix("MT")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Assemble a title, cell block, surface block, and data block into MCNP source.
/// The two blank-line delimiters MCNP requires (cell|surface|data) are always
/// emitted, even when the surface or data block is empty, so the result parses
/// without a "missing delimiter" diagnostic.
fn assemble_blocks(title: &str, cells: &[String], surfaces: &[String], data: &[String]) -> String {
    let mut s = String::new();
    s.push_str(title.trim_end());
    s.push('\n');
    let push_block = |s: &mut String, block: &[String]| {
        for line in block {
            s.push_str(line.trim_end());
            s.push('\n');
        }
    };
    push_block(&mut s, cells);
    s.push('\n'); // cell → surface delimiter
    push_block(&mut s, surfaces);
    s.push('\n'); // surface → data delimiter
    push_block(&mut s, data);
    s
}

/// One id defined by more than one model during a [`Model::merge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeConflict {
    /// Which kind of card the clashing id belongs to.
    pub kind: ConflictKind,
    /// The id defined more than once.
    pub id: i64,
}

impl MergeConflict {
    fn new(kind: ConflictKind, id: i64) -> Self {
        Self { kind, id }
    }
}

impl std::fmt::Display for MergeConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "duplicate {} {} across merged models",
            self.kind, self.id
        )
    }
}

/// The card kind a [`MergeConflict`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    Cell,
    Surface,
    Material,
    Transform,
}

impl std::fmt::Display for ConflictKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ConflictKind::Cell => "cell",
            ConflictKind::Surface => "surface",
            ConflictKind::Material => "material",
            ConflictKind::Transform => "transform",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATTICE: &str = "\
title
1 0 -1 fill=10
2 1 -1.0 -2 3 u=10
3 0 2 -3 u=10

1 SO 5
2 PX -1
3 PX 1

m1 1001 1
";

    #[test]
    fn universe_ids_and_cell_universe() {
        let mut m = Model::parse(LATTICE);
        let v = m.view();
        assert_eq!(v.universe_ids(), vec![10]);
        // Card 0 is the title; cell 1 (card 1) is level-0, cells 2 and 3
        // (cards 2 and 3) are in universe 10.
        assert_eq!(v.cell_universe(0), None); // title card
        assert_eq!(v.cell_universe(1), None); // level-0 cell
        assert_eq!(v.cell_universe(2), Some(10));
        assert_eq!(v.cell_universe(3), Some(10));
    }

    #[test]
    fn extract_universe_keeps_cells_and_referenced_surfaces() {
        let mut m = Model::parse(LATTICE);
        let mut u = m.extract_universe(10);
        {
            let v = u.view();
            // Both universe-10 cells, none of the level-0 cell.
            assert_eq!(v.cells().count(), 2);
            assert!(v.cells().all(|c| c.id == 2 || c.id == 3));
            // Surfaces 2 and 3 are referenced; surface 1 (level-0 only) is dropped.
            let sids: Vec<i64> = v.surfaces().map(|s| s.id).collect();
            assert_eq!(sids, vec![2, 3]);
            // The material used by cell 2 is carried so the extraction runs alone.
            assert_eq!(v.materials().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
        }
        assert!(u.diagnostics().is_empty());
        // ...and clearing it yields the geometry-only sub-model.
        let mut geometry_only = u;
        geometry_only.clear_data_cards();
        assert_eq!(geometry_only.view().data_cards().count(), 0);
    }

    #[test]
    fn view_extract_matches_mut_extract() {
        // `ModelView::extract_universe`/`extract_level0` must agree byte-for-byte
        // with the `&mut self` originals — they share the same `extract_cells`
        // engine, just reached without a fresh materialize.
        let mut via_view = Model::parse(LATTICE);
        let mut via_mut = Model::parse(LATTICE);
        assert_eq!(
            via_view.view().extract_universe(10).to_source(),
            via_mut.extract_universe(10).to_source()
        );
        assert_eq!(
            via_view.view().extract_level0().to_source(),
            via_mut.extract_level0().to_source()
        );
    }

    #[test]
    fn view_extract_universe_runs_across_threads() {
        // `ModelView` borrows the tree immutably, so it is `Sync`: extracting
        // several universes concurrently needs no cloning and no `unsafe`. This
        // is the capability `gitronics` uses to parallelize universe extraction
        // with `rayon` again after `extract_universe` became `&mut self`.
        let src = "\
title
1 0 -1 fill=10
2 1 -1.0 -2 u=10
3 1 -1.0 -2 u=20
4 1 -1.0 -2 u=30

1 SO 5
2 SO 4

m1 1001 1
";
        let mut m = Model::parse(src);
        let view = m.view();
        let view = &view;
        let mut sources: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = view
                .universe_ids()
                .into_iter()
                .map(|u| scope.spawn(move || view.extract_universe(u).to_source()))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        sources.sort();
        assert_eq!(sources.len(), 3);
        assert!(sources[0].contains("u=10"));
        assert!(sources[1].contains("u=20"));
        assert!(sources[2].contains("u=30"));
    }

    #[test]
    fn extract_level0_is_the_inverse_selection() {
        let mut m = Model::parse(LATTICE);
        let mut shell = m.extract_level0();
        let v = shell.view();
        assert_eq!(v.cells().count(), 1);
        assert_eq!(v.cells().next().unwrap().id, 1);
        // The level-0 cell references surface 1 only.
        assert_eq!(v.surfaces().map(|s| s.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn extract_follows_cell_complements_transitively() {
        // Cell 2 (universe 10) complements cell 3, which references surface 4.
        // Extracting universe 10 must pull cell 3 and surface 4 in too.
        let src = "\
title
1 0 -1 fill=10
2 0 -2 #3 u=10
3 0 -4 u=10
4 0 -5 u=20

1 SO 5
2 SO 4
3 SO 3
4 SO 2
5 SO 1

m1 1001 1
";
        let mut m = Model::parse(src);
        let mut u = m.extract_universe(10);
        let v = u.view();
        let mut cids: Vec<i64> = v.cells().map(|c| c.id).collect();
        cids.sort_unstable();
        assert_eq!(cids, vec![2, 3]);
        let sids: Vec<i64> = v.surfaces().map(|s| s.id).collect();
        assert_eq!(sids, vec![2, 4]); // surface 1 (level-0) and 3/5 (u=20) dropped
    }

    #[test]
    fn extract_carries_only_needed_materials_and_transforms() {
        // Cell 2 uses material 1 and surface 2 which carries transform 5.
        // Material 2, transform 9, and their cards are unreferenced; SDEF stays.
        let src = "\
title
1 0 -1 fill=10
2 1 -1.0 -2 u=10

1 SO 5
2 5 SO 4

m1 1001 1
m2 8016 1
tr5 0 0 1
tr9 0 0 2
mt1 lwtr.10t
sdef pos=0 0 0
";
        let mut m = Model::parse(src);
        let mut u = m.extract_universe(10);
        let v = u.view();
        assert_eq!(v.materials().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(v.transforms().map(|t| t.id).collect::<Vec<_>>(), vec![5]);
        let names: Vec<String> = v.data_cards().map(|d| d.name).collect();
        assert!(names.contains(&"M1".to_string()), "{names:?}");
        assert!(names.contains(&"TR5".to_string()), "{names:?}");
        assert!(names.contains(&"MT1".to_string()), "{names:?}");
        assert!(names.contains(&"SDEF".to_string()), "{names:?}");
        // Unreferenced material/transform cards are pruned.
        assert!(!names.contains(&"M2".to_string()), "{names:?}");
        assert!(!names.contains(&"TR9".to_string()), "{names:?}");
        assert!(u.diagnostics().is_empty());
    }

    #[test]
    fn extract_follows_fill_transform_reference() {
        // A nested fill's `(TRn)` reference must carry that transform.
        let src = "\
title
1 0 -1 fill=10
2 0 -2 fill=20 (7) u=10

1 SO 5
2 SO 4

tr7 0 0 3
";
        let mut m = Model::parse(src);
        let mut u = m.extract_universe(10);
        assert_eq!(
            u.view().transforms().map(|t| t.id).collect::<Vec<_>>(),
            vec![7]
        );
    }

    #[test]
    fn extract_reattaches_headers_misattached_to_dropped_cards() {
        // "c banner" trails the title (absorbed backward, per `build_cards`);
        // "c Universe 10 cells" trails the level-0 cell but is really a header
        // for the u=10 cell that follows it. Extracting universe 10 drops the
        // level-0 cell, so that header must travel onto cell 2 instead of
        // being discarded with its host — and the title must not keep the
        // banner that was never meant for the extraction at all.
        let src = "\
title
c banner
c another banner line
1 0 -1 fill=10
c Universe 10 cells
2 0 -2 u=10

1 SO 5
2 SO 4
";
        let mut m = Model::parse(src);
        let u = m.extract_universe(10);
        let out = u.tree.to_source();
        assert!(
            out.contains("c Universe 10 cells\n2 0 -2 u=10"),
            "header did not reattach to the kept cell: {out:?}"
        );
        assert!(
            !out.contains("banner"),
            "banner meant for the dropped cell leaked into the extraction: {out:?}"
        );
    }

    #[test]
    fn merge_appends_disjoint_models() {
        let mut shell = Model::parse("title\n1 0 -1 fill=10\n\n1 SO 5\n\nm1 1001 1\n");
        let filler = Model::parse("f\n2 0 -2 u=10\n\n2 SO 3\n");
        shell.merge(&[&filler]).unwrap();
        let v = shell.view();
        assert_eq!(v.cells().count(), 2);
        assert_eq!(v.surfaces().count(), 2);
        // The data card from the shell survives; both cells re-parse cleanly.
        assert_eq!(v.materials().count(), 1);
        assert!(v.validate().is_empty(), "{:?}", v.validate());
    }

    #[test]
    fn merge_reports_id_collisions_and_leaves_model_untouched() {
        let mut a = Model::parse("title\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\n");
        let before = a.to_source();
        // `b` re-uses cell id 1 and surface id 1 — both must be reported.
        let b = Model::parse("f\n1 0 -1\n\n1 SO 9\n");
        let err = a.merge(&[&b]).unwrap_err();
        assert!(err.contains(&MergeConflict {
            kind: ConflictKind::Cell,
            id: 1
        }));
        assert!(err.contains(&MergeConflict {
            kind: ConflictKind::Surface,
            id: 1
        }));
        // Nothing changed on conflict.
        assert_eq!(a.to_source(), before);
    }

    #[test]
    fn clear_data_cards_keeps_geometry() {
        let mut m = Model::parse(LATTICE);
        assert_eq!(m.view().data_cards().count(), 1); // m1
        m.clear_data_cards();
        {
            let v = m.view();
            assert_eq!(v.data_cards().count(), 0);
            assert_eq!(v.cells().count(), 3);
            assert_eq!(v.surfaces().count(), 3);
        }
        assert!(m.diagnostics().is_empty());
    }

    #[test]
    fn append_inline_comment_is_lossless() {
        let mut m = Model::parse("title\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        m.append_inline_comment(1, "@env:main").unwrap();
        assert!(
            m.to_source().contains("1 0 -1 imp:n=1 $ @env:main"),
            "{}",
            m.to_source()
        );
    }
}
