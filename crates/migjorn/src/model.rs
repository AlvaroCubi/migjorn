//! The `Model`: the single public facade over the per-card CST.

use migjorn_syntax::{Card, CardKind, Cst, SyntaxKind};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::data::{self, DataHead};
use crate::diagnostic::{Diagnostic, Pending, Severity};
use crate::view::{CellView, DataCardView, MaterialView, SurfaceView, TransformView};
use crate::{cell, surface};

/// id -> stable slot. Maintained incrementally by the edit methods, never
/// rebuilt on read.
pub(crate) type IdIndex = FxHashMap<i64, u32>;

/// A parsed MCNP model: lossless, typed access, cheap iterative editing.
#[derive(Clone)]
pub struct Model {
    pub(crate) cst: Cst,
    diagnostics: Vec<Diagnostic>,
    // Maintained incrementally by the structural-edit methods in `edit.rs`, which
    // is why these are crate-visible rather than private to this module.
    pub(crate) cell_index: IdIndex,
    pub(crate) surface_index: IdIndex,
    pub(crate) material_index: IdIndex,
    pub(crate) transform_index: IdIndex,
}

/// Below this many cards, `build_indices` scans sequentially instead of
/// fanning out across the rayon pool.
///
/// A caller that composes many models in one process (parsing each of a
/// project's files, then calling `clear_data_cards` on each) triggers this
/// once per file; below this size, dispatching a `par_chunks` fan-out costs
/// more than the sequential scan it replaces, and that cost is worse still
/// when it is nested inside the caller's own outer parallelism (e.g.
/// `gitronics::load_fillers`), where it just contends with sibling tasks for
/// the same pool. See `docs/05-parallelism-overhead.md`.
const PARALLEL_INDEX_THRESHOLD: usize = 100_000;

impl Model {
    pub fn parse(src: &str) -> Model {
        Model::from_cst(Cst::parse(src))
    }

    /// Build a `Model` from an already-built `Cst` (e.g. one assembled from
    /// existing cards via `Cst::from_cards`, with no lexing of its own).
    pub(crate) fn from_cst(cst: Cst) -> Model {
        let mut model = Model {
            cst,
            diagnostics: Vec::new(),
            cell_index: IdIndex::default(),
            surface_index: IdIndex::default(),
            material_index: IdIndex::default(),
            transform_index: IdIndex::default(),
        };
        model.build_indices();
        model
    }

    /// Re-emit. Byte-identical to the input when unedited.
    pub fn to_source(&self) -> String {
        self.cst.to_source()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn cst(&self) -> &Cst {
        &self.cst
    }

    // --- iteration & lookup -------------------------------------------------

    /// The model's title line, if one exists — the single line immediately
    /// following an optional leading `MESSAGE` block (`CardKind::Title`, see
    /// `segment.rs`). `None` for a model with no title card (e.g. empty
    /// input). Never at a fixed CST position — a leading `MESSAGE` block
    /// displaces it.
    pub fn title(&self) -> Option<&str> {
        let text = self
            .cst
            .cards()
            .find(|c| c.kind() == CardKind::Title)?
            .text();
        Some(strip_eol(text))
    }

    pub fn cells(&self) -> impl Iterator<Item = CellView<'_>> + '_ {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Cell)
            .map(move |c| CellView::new(self, c.slot()))
    }

    pub fn surfaces(&self) -> impl Iterator<Item = SurfaceView<'_>> + '_ {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Surface)
            .map(move |c| SurfaceView::new(self, c.slot()))
    }

    pub fn materials(&self) -> impl Iterator<Item = MaterialView<'_>> + '_ {
        self.data_heads()
            .filter(|(_, h)| data::material_id(h).is_some())
            .map(move |(slot, _)| MaterialView::new(self, slot))
    }

    pub fn transforms(&self) -> impl Iterator<Item = TransformView<'_>> + '_ {
        self.data_heads()
            .filter(|(_, h)| data::transform_id(h).is_some())
            .map(move |(slot, _)| TransformView::new(self, slot))
    }

    pub fn data_cards(&self) -> impl Iterator<Item = DataCardView<'_>> + '_ {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Data)
            .map(move |c| DataCardView::new(self, c.slot()))
    }

    fn data_heads(&self) -> impl Iterator<Item = (u32, DataHead)> + '_ {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Data)
            .filter_map(|c| data::head(c).map(|h| (c.slot(), h)))
    }

    pub fn num_cells(&self) -> usize {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Cell)
            .count()
    }

    pub fn num_surfaces(&self) -> usize {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Surface)
            .count()
    }

    pub fn num_materials(&self) -> usize {
        self.materials().count()
    }

    pub fn num_transforms(&self) -> usize {
        self.transforms().count()
    }

    /// O(1) lookup through the maintained id index.
    pub fn cell(&self, id: i64) -> Option<CellView<'_>> {
        let slot = *self.cell_index.get(&id)?;
        self.cst.card(slot).map(|_| CellView::new(self, slot))
    }

    pub fn surface(&self, id: i64) -> Option<SurfaceView<'_>> {
        let slot = *self.surface_index.get(&id)?;
        self.cst.card(slot).map(|_| SurfaceView::new(self, slot))
    }

    pub fn material(&self, id: i64) -> Option<MaterialView<'_>> {
        let slot = *self.material_index.get(&id)?;
        self.cst.card(slot).map(|_| MaterialView::new(self, slot))
    }

    pub fn transform(&self, id: i64) -> Option<TransformView<'_>> {
        let slot = *self.transform_index.get(&id)?;
        self.cst.card(slot).map(|_| TransformView::new(self, slot))
    }

    pub(crate) fn card(&self, slot: u32) -> Option<&Card> {
        self.cst.card(slot)
    }

    // --- views by stable slot (the anchor a live handle stores) --------------

    /// A cell view for a stable slot, or `None` if that slot was removed or is
    /// not a cell. This is how a language binding resolves a live handle each use.
    pub fn cell_at(&self, slot: u32) -> Option<CellView<'_>> {
        self.kind_at(slot, CardKind::Cell)
            .map(|_| CellView::new(self, slot))
    }

    pub fn surface_at(&self, slot: u32) -> Option<SurfaceView<'_>> {
        self.kind_at(slot, CardKind::Surface)
            .map(|_| SurfaceView::new(self, slot))
    }

    /// A material view for a slot: the slot must be a live `Mn` data card.
    pub fn material_at(&self, slot: u32) -> Option<MaterialView<'_>> {
        let card = self.cst.card(slot)?;
        (card.kind() == CardKind::Data
            && data::head(card)
                .and_then(|h| data::material_id(&h))
                .is_some())
        .then(|| MaterialView::new(self, slot))
    }

    pub fn transform_at(&self, slot: u32) -> Option<TransformView<'_>> {
        let card = self.cst.card(slot)?;
        (card.kind() == CardKind::Data
            && data::head(card)
                .and_then(|h| data::transform_id(&h))
                .is_some())
        .then(|| TransformView::new(self, slot))
    }

    /// A generic data-card view for a slot — a superset of `material_at`/
    /// `transform_at` (doesn't distinguish `Mn`/`TRn` from anything else).
    pub fn data_card_at(&self, slot: u32) -> Option<DataCardView<'_>> {
        self.kind_at(slot, CardKind::Data)
            .map(|_| DataCardView::new(self, slot))
    }

    fn kind_at(&self, slot: u32, kind: CardKind) -> Option<()> {
        (self.cst.card(slot)?.kind() == kind).then_some(())
    }

    // --- index construction -------------------------------------------------

    /// Build the four id indices and the parse diagnostics in one parallel pass.
    ///
    /// Scanning is chunked rather than per-card: a chunk fills one `ids` vector
    /// for its whole slice, so a million cards cost a few dozen allocations
    /// instead of a few million. That difference is most of the index-building
    /// time on a large model.
    fn build_indices(&mut self) {
        let order = self.cst.order();
        // Below the threshold, one chunk covering the whole model: the
        // `par_chunks` pass below then runs inline with no rayon dispatch at
        // all, rather than fanning out across the pool for a scan that is
        // cheaper to just do sequentially.
        let chunk = if order.len() < PARALLEL_INDEX_THRESHOLD {
            order.len().max(1)
        } else {
            (order.len() / (rayon::current_num_threads().max(1) * 4)).max(1)
        };
        let cst = &self.cst;

        let scanned: Vec<Scan> = order
            .par_chunks(chunk)
            .enumerate()
            .map(|(ci, slots)| {
                let mut scan = Scan {
                    ids: Vec::with_capacity(slots.len()),
                    diagnostics: Vec::new(),
                };
                for (k, &slot) in slots.iter().enumerate() {
                    if let Some(card) = cst.card(slot) {
                        scan_card(card, ci * chunk + k, slot, &mut scan);
                    }
                }
                scan
            })
            .collect();

        // Only pay for the card-offset table if something needs a span.
        let needs_offsets = scanned.iter().any(|s| !s.diagnostics.is_empty());
        let offsets = needs_offsets.then(|| {
            let mut offsets = Vec::with_capacity(order.len());
            let mut at = 0usize;
            for &slot in order {
                offsets.push(at);
                at += cst.card(slot).map_or(0, Card::len_bytes);
            }
            offsets
        });

        let mut counts = [0usize; 4];
        for scan in &scanned {
            for (kind, ..) in &scan.ids {
                counts[*kind as usize] += 1;
            }
        }
        self.cell_index.reserve(counts[Kind::Cell as usize]);
        self.surface_index.reserve(counts[Kind::Surface as usize]);
        self.material_index.reserve(counts[Kind::Material as usize]);
        self.transform_index
            .reserve(counts[Kind::Transform as usize]);

        let mut duplicates = Vec::new();
        for scan in &scanned {
            for &(kind, id, slot) in &scan.ids {
                let (index, label) = match kind {
                    Kind::Cell => (&mut self.cell_index, "cell"),
                    Kind::Surface => (&mut self.surface_index, "surface"),
                    Kind::Material => (&mut self.material_index, "material"),
                    Kind::Transform => (&mut self.transform_index, "transform"),
                };
                // The first definition wins the index; the duplicate is reported.
                if index.insert(id, slot).is_some() {
                    duplicates.push((label, id));
                }
            }
        }
        for (label, id) in duplicates {
            self.diagnostics.push(Diagnostic::error(
                format!("duplicate {label} id {id}"),
                0..0,
            ));
        }

        for scan in scanned {
            for pending in scan.diagnostics {
                let base = offsets.as_ref().map_or(0, |o| o[pending.card]);
                let span = base + pending.local.start..base + pending.local.end;
                self.diagnostics.push(match pending.severity {
                    Severity::Error => Diagnostic::error(pending.message, span),
                    Severity::Warning => Diagnostic::warning(pending.message, span),
                });
            }
        }
    }
}

/// Trailing `\r\n` / `\n` removed, if present.
fn strip_eol(text: &str) -> &str {
    text.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(text)
}

/// One chunk's worth of scan results.
struct Scan {
    ids: Vec<(Kind, i64, u32)>,
    diagnostics: Vec<Pending>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
enum Kind {
    Cell = 0,
    Surface = 1,
    Material = 2,
    Transform = 3,
}

/// Read one card's defined ids and any problems with it.
fn scan_card(card: &Card, index: usize, slot: u32, out: &mut Scan) {
    // The flag is set at lex time, so the overwhelmingly common case costs one
    // predictable branch rather than a walk over the card's tokens.
    if card.has_unknown() {
        for token in card.tokens() {
            if token.kind == SyntaxKind::Unknown {
                out.diagnostics.push(Pending {
                    card: index,
                    local: token.range(),
                    severity: Severity::Warning,
                    message: format!("unrecognized token `{}`", &card.text()[token.range()]),
                });
            }
        }
    }

    match card.kind() {
        CardKind::Cell => {
            let l = cell::layout(card);
            match l.id {
                Some(id) => out.ids.push((Kind::Cell, id, slot)),
                None => out.diagnostics.push(Pending {
                    card: index,
                    local: 0..card.len_bytes(),
                    severity: Severity::Error,
                    message: "cell card has no readable id".to_owned(),
                }),
            }
            if !l.well_formed {
                out.diagnostics.push(Pending {
                    card: index,
                    local: 0..card.len_bytes(),
                    severity: Severity::Warning,
                    message: match l.id {
                        Some(id) => format!("cell {id} is not well formed"),
                        None => "cell is not well formed".to_owned(),
                    },
                });
            }
        }
        CardKind::Surface => {
            let l = surface::layout(card);
            match l.id {
                Some(id) => out.ids.push((Kind::Surface, id, slot)),
                None => out.diagnostics.push(Pending {
                    card: index,
                    local: 0..card.len_bytes(),
                    severity: Severity::Error,
                    message: "surface card has no readable id".to_owned(),
                }),
            }
            if !l.well_formed {
                out.diagnostics.push(Pending {
                    card: index,
                    local: 0..card.len_bytes(),
                    severity: Severity::Warning,
                    message: match l.id {
                        Some(id) => format!("surface {id} is not well formed"),
                        None => "surface is not well formed".to_owned(),
                    },
                });
            }
        }
        CardKind::Data => {
            if let Some(head) = data::head(card) {
                if let Some(id) = data::material_id(&head) {
                    out.ids.push((Kind::Material, id, slot));
                    let (_, ok) = data::material_entries(card, &head);
                    if !ok {
                        out.diagnostics.push(Pending {
                            card: index,
                            local: 0..card.len_bytes(),
                            severity: Severity::Warning,
                            message: format!("material {id} has an unreadable entry"),
                        });
                    }
                } else if let Some(id) = data::transform_id(&head) {
                    out.ids.push((Kind::Transform, id, slot));
                }
            }
        }
        _ => {}
    }
}
