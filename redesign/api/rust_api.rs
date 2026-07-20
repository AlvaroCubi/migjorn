//! Sketch of the target Rust API and the core data model for the rewrite.
//!
//! This is a DESIGN SKETCH — signatures and the shape of the types, with bodies
//! left as `todo!()`. It exists to pin down the architecture in code form so the
//! implementation has a concrete target. The guiding decision:
//!
//!   **Cards are the atomic unit.** The model is a `Vec<Card>` where each card
//!   uniformly owns its own text (`String`) and token buffer (`Vec<Token>`).
//!   Structural edits are `Vec<Card>` operations; in-card edits mutate that one
//!   card's text and tokens; emission concatenates every card's text. There is
//!   no borrowed/owned distinction and no edit overlay — every card is handled
//!   identically regardless of its edit history. Nothing ever re-lexes the file.
//!
//! Crate layout is unchanged from today (it is good):
//!   migjorn-syntax : lexer + per-card CST (this file's `Cst`, `Card`, `Token`)
//!   migjorn        : typed model + editing + renumber + compose (the `Model`)
//!   migjorn-py     : PyO3 bindings (abi3), mirroring migjorn.pyi
//!
//! See docs/02-architecture.md for the rationale and the measured numbers that
//! motivate this shape.

use std::ops::Range;

// ===========================================================================
// migjorn-syntax : the lossless, per-card concrete syntax layer
// ===========================================================================

/// Lexical categories. MCNP tokens tile the source exactly (including trivia),
/// so a card's text is recoverable by concatenating its tokens — the property
/// that makes losslessness a memcpy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    Number,
    Ident,
    Colon,
    Eq,
    Star,
    Plus,
    Hash,
    LParen,
    RParen,
    // trivia (preserved, never dropped):
    Whitespace,
    Newline,
    Comment,      // full-line `c`/`C` or inline `$...`
    Ampersand,    // `&` continuation marker
    Unknown,      // recoverability: any byte run we don't classify
}

impl SyntaxKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::Comment | SyntaxKind::Ampersand
        )
    }
}

/// A single token: its kind and its byte span **within its owning card's text**.
/// Text is a span (relative to the card), never an owned string — a card carries
/// tens of tokens, not the whole file, so its tokens tile its own `String`.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: SyntaxKind,
    /// Byte range within the owning card's `text`.
    pub span: Range<u32>,
}

/// What kind of card this is, decided cheaply at lex time from block position
/// and the leading tokens. `Comment`/`Blank` are structural trivia cards that
/// carry text but no typed meaning; keeping them as cards is what makes
/// insert/remove preserve layout exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    Title,
    Cell,
    Surface,
    Data,
    Comment,
    Blank,
    Message,
}

/// One card: the atomic unit of the whole design.
///
/// **Every card owns its bytes and its tokens — uniformly, from parse onward.**
/// There is no borrowed/owned split and no edit overlay: a card is always a
/// `String` (its exact current text) plus the `Vec<Token>` that tiles it. This
/// is the deliberate simplicity of the design — every card is handled the same
/// way regardless of whether or how it has been edited (see
/// docs/02-architecture.md "Why uniform ownership" for the measurement that
/// justifies it: the shared-arena alternative saves ~0.9 s of parse time and
/// nothing else, so it is left as an optional, deferrable optimization).
///
/// - Emit a card: `out.push_str(&card.text)` — a memcpy, edited or not, because
///   an unedited card's `text` already holds its original bytes exactly.
/// - Edit a token (renumber, set value): splice the new text into `text` and
///   shift the following token spans. O(card length) — a card is tens of tokens.
/// - Structural in-card edit (add/remove a parameter, splice a surface): same —
///   rewrite `text` + `tokens`. Nothing special, no mode change.
pub struct Card {
    /// Stable id that survives inserts/removes — the anchor a live handle
    /// resolves through, so a handle is never invalidated by an edit to a
    /// *different* card.
    pub slot: u32,
    pub kind: CardKind,
    /// This card's exact current bytes. Initially a copy of its source slice;
    /// mutated in place by edits. Emission is a straight copy of this.
    text: String,
    /// Tokens tiling `text` (spans relative to `text`), trivia included.
    tokens: Vec<Token>,
}

/// The concrete syntax tree: just the ordered list of cards. Each card is
/// self-contained, so there is no shared source buffer or token arena to keep in
/// sync — structural editing is a plain `Vec<Card>` operation, and the original
/// source is not retained after parse.
pub struct Cst {
    cards: Vec<Card>,
}

impl Cst {
    /// Lex `src` into self-contained cards (each copies its bytes + builds its
    /// tokens). Cards are independent, so this is embarrassingly parallel: split
    /// into blocks, lex+build cards in parallel, concatenate. Target: <= 1.0 s
    /// on a 380 MB / ~1M-card file — the per-card copy costs ~0.9 s
    /// single-threaded, so parallel construction is what keeps parse in budget
    /// (see docs/04). The input `&str` is only borrowed during parse; it need
    /// not be retained.
    pub fn parse(src: &str) -> Cst {
        todo!()
    }

    pub fn cards(&self) -> &[Card] {
        todo!()
    }

    /// Text of one card, reflecting its edits — a slice of the card's own
    /// buffer. O(1).
    pub fn card_text(&self, i: usize) -> &str {
        todo!()
    }

    /// Emit the whole model: `for card in cards { out.push_str(&card.text) }`.
    /// O(bytes), no reparse, byte-identical to the input when unedited. There is
    /// no fast/slow path — every card emits the same way whether or not it was
    /// edited.
    pub fn to_source(&self) -> String {
        todo!()
    }

    // --- structural editing: pure `Vec<Card>` operations --------------------

    /// Insert an already-built card at position `i`; returns its fresh slot.
    /// O(num_cards) memmove of card structs — no relex of the file.
    pub fn insert_card(&mut self, i: usize, card: Card) -> u32 {
        todo!()
    }

    /// Remove the card at `i`. O(num_cards). Its slot never reappears.
    pub fn remove_card(&mut self, i: usize) {
        todo!()
    }

    /// Lex a single snippet into a detached, self-contained `Card` (used by
    /// `insert_card`). O(snippet length) — microseconds.
    pub fn lex_card(text: &str, kind: CardKind) -> Card {
        todo!()
    }

    // --- in-card editing: mutate the card's own text + tokens ---------------

    /// Overwrite one token's text in place: splice `text` into the card's buffer
    /// at that token's span and shift the following spans by the length delta.
    /// This is how renumber and value edits work — every card is edited the same
    /// way, whether it is being touched for the first time or the hundredth.
    /// O(card length).
    pub fn set_token_text(&mut self, card: usize, token: usize, text: &str) {
        todo!()
    }

    /// Structural in-card edit: splice tokens in/out (add a surface to geometry,
    /// add/remove a parameter) by rewriting a byte range of the card's `text`
    /// and re-tokenizing just that card. O(card length) — same cost class as a
    /// value edit; no distinct code path.
    pub fn splice_card_text(&mut self, card: usize, byte_range: Range<usize>, text: &str) {
        todo!()
    }

    /// Resolve a stable slot to a current card index (None if removed).
    pub fn card_of_slot(&self, slot: u32) -> Option<usize> {
        todo!()
    }
}

// ===========================================================================
// migjorn : the typed model, editing, renumbering, composition
// ===========================================================================

/// Old id -> new id. `renumber_*` accept any `FnMut(i64) -> i64`; the Python
/// binding adapts a dict or callable into one.
pub trait IdMap: FnMut(i64) -> i64 {}
impl<F: FnMut(i64) -> i64> IdMap for F {}

#[derive(Debug)]
pub enum EditError {
    NotOneCard,
    WrongKind,
    WouldEmptyGeometry,
    NoBlock,
}

/// The single public facade. Owns a `Cst`, maintained id → slot indices for O(1)
/// lookup, and the diagnostics from the last parse. Every id index is kept in
/// sync incrementally by the edit methods (no rebuild-on-read).
pub struct Model {
    cst: Cst,
    diagnostics: Vec<Diagnostic>,
    cell_index: IdIndex,
    surface_index: IdIndex,
    material_index: IdIndex,
    transform_index: IdIndex,
}

/// id -> stable slot, kept current across edits.
type IdIndex = rustc_hash::FxHashMap<i64, u32>;

pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Range<usize>,
}
pub enum Severity {
    Error,
    Warning,
}

impl Model {
    pub fn parse(src: &str) -> Model {
        todo!()
    }
    pub fn to_source(&self) -> String {
        todo!()
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        todo!()
    }

    // --- typed, on-demand projection (no allocation until asked) ------------
    pub fn cells(&self) -> impl Iterator<Item = CellView<'_>> {
        std::iter::empty()
    }
    pub fn cell(&self, id: i64) -> Option<CellView<'_>> {
        todo!()
    }
    pub fn surface(&self, id: i64) -> Option<SurfaceView<'_>> {
        todo!()
    }
    pub fn num_cells(&self) -> usize {
        todo!()
    }

    // --- structural editing (local; no whole-file reparse) ------------------
    pub fn add_cell(&mut self, text: &str) -> Result<u32, EditError> {
        todo!()
    }
    pub fn remove_cell(&mut self, id: i64) -> Result<bool, EditError> {
        todo!()
    }

    // --- in-card value edits (addressed by stable slot) ---------------------
    pub fn set_cell_material(&mut self, slot: u32, material: i64) -> Result<(), EditError> {
        todo!()
    }

    // --- whole-model renumbering (definitions + every reference) ------------
    pub fn renumber_cells(&mut self, map: impl IdMap) {
        todo!()
    }
    pub fn renumber_surfaces(&mut self, map: impl IdMap) {
        todo!()
    }
    // ... materials / transforms / universes / tallies

    // --- composition --------------------------------------------------------
    pub fn validate(&self) -> Vec<String> {
        todo!()
    }
    pub fn extract_universe(&self, u: i64) -> Model {
        todo!()
    }
    pub fn merge(&mut self, others: Vec<Model>) -> Result<(), Vec<String>> {
        todo!()
    }
}

/// A borrowed, typed view over one cell card, projected on demand from its
/// tokens. Reads are `&self`; writes go through `Model` addressed by slot (so a
/// view never holds a `&mut` that would forbid reading two cells at once).
pub struct CellView<'a> {
    model: &'a Model,
    slot: u32,
}
pub struct SurfaceView<'a> {
    model: &'a Model,
    slot: u32,
}
