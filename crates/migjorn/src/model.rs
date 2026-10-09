//! The `Model`: the single public facade over the per-card CST.

use migjorn_syntax::{Card, CardKind, Cst, SyntaxKind};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::fmt;
use std::io;
use std::ops::Range;

use crate::data::{self, DataHead};
use crate::diagnostic::{Diagnostic, Pending, Severity};
use crate::scan::byte_span;
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

/// A summary, not the whole CST — nobody wants a multi-hundred-MB model dumped
/// into a panic message or an `assert_eq!` failure. Enough to make
/// `Result<_, Model>`/`Vec<Model>` usable in a test: `unwrap_err()`,
/// `assert_eq!`, `#[derive(Debug)]` on a wrapper type.
impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("title", &self.title())
            .field("cards", &self.cst.len())
            .field("diagnostics", &self.diagnostics.len())
            .finish()
    }
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

    /// A model whose only content cards are data cards — the constructor
    /// counterpart to [`Model::clear_data_cards`]. Assembling one to `merge`
    /// in (e.g. a project's own configured data cards, folded in once
    /// alongside its cell/surface fillers) means placing `text` positionally
    /// so it lands in the data block: an empty cell block, then an empty
    /// surface block, then `text`. Getting that placement wrong by even one
    /// newline used to be a silent miscategorisation a caller had to get
    /// right by hand — one line short and a `data` card like `M1` becomes a
    /// `Surface` card instead, which `validate()` does not catch (only
    /// `diagnostics()` does, as an easy-to-miss "surface card has no readable
    /// id" error) — so it is built correctly here once instead of at every
    /// call site.
    pub fn from_data_cards(text: &str) -> Model {
        let mut src = String::with_capacity(text.len() + 16);
        src.push_str("data cards\n\n\n");
        src.push_str(text);
        if !src.ends_with('\n') {
            src.push('\n');
        }
        Model::parse(&src)
    }

    /// Re-emit. Byte-identical to the input when unedited.
    pub fn to_source(&self) -> String {
        self.cst.to_source()
    }

    /// Stream the source to `w` one card at a time, without allocating the
    /// whole thing as one `String` first the way `to_source` does — worth it
    /// once a model is large enough that the extra copy matters. Byte-
    /// identical to `to_source`'s output, since emission is a plain
    /// concatenation of each card's own bytes either way.
    pub fn write_source(&self, w: &mut impl io::Write) -> io::Result<()> {
        self.cst.write_source(w)
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

    /// The title card's stable slot, if one exists — the usual anchor for
    /// [`Model::insert_card_after`] (e.g. a provenance banner placed right
    /// after the title).
    pub fn title_slot(&self) -> Option<u32> {
        self.cst
            .cards()
            .find(|c| c.kind() == CardKind::Title)
            .map(|c| c.slot())
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

    // --- source locations ---------------------------------------------------

    /// The 1-based line, in [`Model::to_source`] as it reads now, of the first
    /// token of the card at `slot` (comment lines above it not counted).
    /// `None` if the slot was removed.
    ///
    /// A pass over the text up to the card; see [`Model::card_lines`] to
    /// place many cards.
    pub fn line_of(&self, slot: u32) -> Option<usize> {
        let mut line = 1usize;
        for &s in self.cst.order() {
            let card = self.cst.card(s)?;
            if s == slot {
                return Some(line + first_token_line(card));
            }
            line += newlines(card.text());
        }
        None
    }

    /// The line of every card, in one pass: what [`Model::line_of`] returns,
    /// for all slots at once. A snapshot: an edit that adds or removes a line
    /// moves the cards after it, so build a new table after editing.
    pub fn card_lines(&self) -> CardLines {
        let order = self.cst.order();
        let len = order.iter().max().map_or(0, |&s| s as usize + 1);
        let mut lines = vec![0u32; len];
        let mut line = 1usize;
        for &s in order {
            if let Some(card) = self.cst.card(s) {
                lines[s as usize] =
                    u32::try_from(line + first_token_line(card)).unwrap_or(u32::MAX);
                line += newlines(card.text());
            }
        }
        CardLines { lines }
    }

    // --- cell parameters outside cell cards -----------------------------------

    /// Data cards that set or change what the cell cards say, which the
    /// [`CellView`] getters do not read: the cell-parameter data cards `IMP`,
    /// `U`, `FILL`, `TRCL` and `LAT` (starred or not), a vertical-format
    /// card (`#` in the first columns) with one of them as a column, and
    /// `READ` cards (which may bring in any of these).
    ///
    /// In a model with any of these, `CellView::importance`, `universe`,
    /// `fill_spec`, `trcl` and `lattice` may not be what MCNP uses.
    pub fn cell_data_cards(&self) -> impl Iterator<Item = DataCardView<'_>> + '_ {
        self.cst
            .cards()
            .filter(|c| c.kind() == CardKind::Data)
            .filter(|&c| match data::head(c) {
                Some(h) => {
                    h.number.is_none() && (is_cell_param(&h.mnemonic) || h.mnemonic == "read")
                }
                None => vertical_names_cell_param(c),
            })
            .map(move |c| DataCardView::new(self, c.slot()))
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
                    duplicates.push((label, id, slot));
                }
            }
        }
        for (label, id, slot) in duplicates {
            self.diagnostics
                .push(Diagnostic::error(format!("duplicate {label} id {id}"), 0..0).on_slot(slot));
        }

        for scan in scanned {
            for pending in scan.diagnostics {
                let base = offsets.as_ref().map_or(0, |o| o[pending.card]);
                self.diagnostics.push(pending.at(base));
            }
        }
    }
}

/// The line of every card, by slot; see [`Model::card_lines`].
#[derive(Debug, Clone)]
pub struct CardLines {
    /// Indexed by slot; 0 for a slot that is not in the model.
    lines: Vec<u32>,
}

impl CardLines {
    /// The 1-based line of the card at `slot`, or `None` if the slot was not
    /// in the model when the table was built.
    pub fn line(&self, slot: u32) -> Option<usize> {
        match self.lines.get(slot as usize) {
            Some(&0) | None => None,
            Some(&line) => Some(line as usize),
        }
    }
}

/// A cell parameter that can also be given on a data card, and that the
/// `CellView` getters read from the cell card only.
fn is_cell_param(mnemonic: &str) -> bool {
    matches!(mnemonic, "imp" | "u" | "fill" | "trcl" | "lat")
}

/// A vertical-format data card (`#  imp:n  u …`, then one row per cell) with
/// a cell parameter among its column names.
fn vertical_names_cell_param(card: &Card) -> bool {
    let Some(first) = crate::scan::sig(card, 0) else {
        return false;
    };
    if card.tokens()[first].kind != SyntaxKind::Hash {
        return false;
    }
    // Column names are identifiers; a particle after `:` is not one.
    (first + 1..card.tokens().len()).any(|i| {
        card.tokens()[i].kind == SyntaxKind::Ident
            && crate::scan::prev(card, i).is_none_or(|j| card.tokens()[j].kind != SyntaxKind::Colon)
            && is_cell_param(
                &card
                    .token_text(i)
                    .trim_start_matches('*')
                    .to_ascii_lowercase(),
            )
    })
}

fn newlines(text: &str) -> usize {
    memchr::memchr_iter(b'\n', text.as_bytes()).count()
}

/// Lines before the card's first token, within the card (comment lines that
/// belong to it).
fn first_token_line(card: &Card) -> usize {
    match crate::scan::sig(card, 0) {
        Some(i) => newlines(&card.text()[..card.tokens()[i].start as usize]),
        None => 0,
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
    let defined = inspect(card, |local, severity, message| {
        out.diagnostics.push(Pending {
            card: index,
            slot,
            local,
            severity,
            message,
        })
    });
    if let Some((kind, id)) = defined {
        out.ids.push((kind, id, slot));
    }
}

/// Problems with one card as it reads now, with spans relative to its text.
/// This is what the parse diagnostics hold for the card, recomputed.
pub(crate) fn card_diagnostics(card: &Card, slot: u32) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    inspect(card, |local, severity, message| {
        out.push(
            Pending {
                card: 0,
                slot,
                local,
                severity,
                message,
            }
            .at(0),
        )
    });
    out
}

/// The id a card defines, if any, with every problem found in it reported to
/// `issue` as `(byte span in the card, severity, message)`.
fn inspect(
    card: &Card,
    mut issue: impl FnMut(Range<usize>, Severity, String),
) -> Option<(Kind, i64)> {
    let whole = 0..card.len_bytes();
    // The flag is set at lex time, so the overwhelmingly common case costs one
    // predictable branch rather than a walk over the card's tokens.
    if card.has_unknown() {
        for token in card.tokens() {
            if token.kind == SyntaxKind::Unknown {
                issue(
                    token.range(),
                    Severity::Warning,
                    format!("unrecognized token `{}`", &card.text()[token.range()]),
                );
            }
        }
    }
    let span = |toks: Range<usize>| byte_span(card, toks);

    match card.kind() {
        CardKind::Cell => {
            let l = cell::layout(card);
            let name = match l.id {
                Some(id) => format!("cell {id}"),
                None => {
                    issue(
                        whole.clone(),
                        Severity::Error,
                        "cell card has no readable id".to_owned(),
                    );
                    "cell".to_owned()
                }
            };
            if !l.well_formed {
                issue(
                    whole,
                    Severity::Warning,
                    format!("{name} is not well formed"),
                );
            }
            for (toks, message) in cell::geometry_problems(card, &l.geometry)
                .into_iter()
                .chain(cell::param_problems(card, &l.params))
            {
                issue(span(toks), Severity::Warning, format!("{name}: {message}"));
            }
            l.id.map(|id| (Kind::Cell, id))
        }
        CardKind::Surface => {
            let l = surface::layout(card);
            let name = match l.id {
                Some(id) => format!("surface {id}"),
                None => {
                    issue(
                        whole.clone(),
                        Severity::Error,
                        "surface card has no readable id".to_owned(),
                    );
                    "surface".to_owned()
                }
            };
            if !l.well_formed {
                issue(
                    whole,
                    Severity::Warning,
                    format!("{name} is not well formed"),
                );
            }
            if let Some((toks, message)) = surface::coeff_problem(card, &l) {
                issue(span(toks), Severity::Warning, format!("{name}: {message}"));
            }
            l.id.map(|id| (Kind::Surface, id))
        }
        CardKind::Data => {
            let head = data::head(card)?;
            if let Some(id) = data::material_id(&head) {
                let (_, ok) = data::material_entries(card, &head);
                if !ok {
                    issue(
                        whole,
                        Severity::Warning,
                        format!("material {id} has an unreadable entry"),
                    );
                }
                Some((Kind::Material, id))
            } else if let Some(id) = data::transform_id(&head) {
                if let Some((toks, message)) = data::transform_problem(card, &head) {
                    issue(
                        span(toks),
                        Severity::Warning,
                        format!("transform {id}: {message}"),
                    );
                }
                Some((Kind::Transform, id))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Model;

    #[test]
    fn debug_is_a_summary_not_the_whole_cst() {
        let m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let out = format!("{m:?}");
        assert!(out.contains("Model"));
        assert!(out.contains("title"));
        assert!(!out.contains("1 0 -1 imp:n=1"), "{out}");
    }

    #[test]
    fn write_source_matches_to_source() {
        let src = "t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n";
        let m = Model::parse(src);
        let mut buf = Vec::new();
        m.write_source(&mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), m.to_source());
        assert_eq!(m.to_source(), src);
    }

    #[test]
    fn from_data_cards_lands_everything_in_the_data_block() {
        let m = Model::from_data_cards("sdef pos=0 0 0\nm1 1001 1\n");
        assert_eq!(m.num_cells(), 0);
        assert_eq!(m.num_surfaces(), 0);
        assert!(m.material(1).is_some());
        assert!(m.data_cards().any(|d| d.text().starts_with("sdef")));
        assert!(m.validate().is_empty());
        assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    }

    #[test]
    fn from_data_cards_normalises_a_missing_trailing_newline() {
        let m = Model::from_data_cards("sdef pos=0 0 0");
        assert!(m.to_source().ends_with("sdef pos=0 0 0\n"));
    }

    #[test]
    fn title_slot_resolves_to_the_title_card() {
        let m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let slot = m.title_slot().unwrap();
        assert_eq!(m.cst().card(slot).unwrap().text(), "t\n");

        assert_eq!(Model::parse("").title_slot(), None);
    }
}
