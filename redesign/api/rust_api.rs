//! Sketch of the target Rust API and the core data model for the rewrite.
//!
//! This is a DESIGN SKETCH — signatures and the shape of the types, with bodies
//! left as `todo!()`. It exists to pin down the architecture in code form so the
//! implementation has a concrete target. The guiding decision:
//!
//!   **Cards are the atomic unit.** The model is a `Vec<Card>` where each card
//!   owns (or borrows, when pristine) its own small token buffer. Structural
//!   edits are `Vec<Card>` operations; in-card edits are local token overrides;
//!   emission copies untouched cards verbatim and renders only edited ones.
//!   Nothing ever re-lexes the whole file.
//!
//! Crate layout is unchanged from today (it is good):
//!   migjorn-syntax : lexer + per-card CST (this file's `Cst`, `Card`, `Token`)
//!   migjorn        : typed model + editing + renumber + compose (the `Model`)
//!   migjorn-py     : PyO3 bindings (abi3), mirroring migjorn.pyi
//!
//! See docs/02-architecture.md for the rationale and the measured numbers that
//! motivate this shape.

use std::ops::Range;
use std::sync::Arc;

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

/// A single token: its kind and its byte span into the card's text. Text is a
/// span, never an owned string — no per-token allocation.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: SyntaxKind,
    /// Byte range within the owning card's current text.
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
/// Pristine cards (from the original parse) borrow their text as a span into the
/// shared source and their tokens as a slice of the shared token arena — zero
/// per-card allocation. The moment a card is edited or newly constructed it
/// switches to owning a `String` + `Vec<Token>`. This hybrid keeps parse cheap
/// and edits local.
pub struct Card {
    /// Stable id that survives inserts/removes/reparses — the anchor a live
    /// handle resolves through, so a handle is never invalidated by an edit to
    /// a *different* card.
    pub slot: u32,
    pub kind: CardKind,
    text: CardText,
}

enum CardText {
    /// Untouched since parse: text is `source[span]`, tokens are `arena[range]`.
    Pristine { span: Range<usize>, tokens: Range<usize> },
    /// Edited or freshly built: owns its bytes and tokens.
    Owned { text: String, tokens: Vec<Token> },
}

/// The concrete syntax tree: an immutable shared source snapshot + a shared
/// pristine token arena + the ordered list of cards. Structural editing mutates
/// only `cards`; `source`/`arena` are never mutated (removed/edited cards simply
/// stop pointing into them). `compact()` rebuilds to reclaim space when wanted.
pub struct Cst {
    source: Arc<str>,
    arena: Arc<[Token]>,
    cards: Vec<Card>,
}

impl Cst {
    /// Lex `src` into cards. Cards are independent, so this is embarrassingly
    /// parallel: split into blocks, lex each in parallel, concatenate. Target:
    /// <= 1.0s single-threaded, <= 0.5s parallel, on a 380 MB / ~1M-card file.
    pub fn parse(src: impl Into<Arc<str>>) -> Cst {
        todo!()
    }

    pub fn cards(&self) -> &[Card] {
        todo!()
    }

    /// Text of one card, reflecting its edits. Pristine → a `&str` slice;
    /// owned → its buffer. O(card length).
    pub fn card_text(&self, i: usize) -> &str {
        todo!()
    }

    /// Emit the whole model. Untouched cards are memcpy'd from `source`; edited
    /// cards are rendered from their owned tokens. O(bytes) + O(edited cards),
    /// with no reparse. Byte-identical to the input when unedited.
    pub fn to_source(&self) -> String {
        todo!()
    }

    // --- structural editing: pure `Vec<Card>` operations --------------------

    /// Insert an already-lexed card at position `i`; returns its fresh slot.
    /// O(num_cards) memmove of card structs — no relex of the file.
    pub fn insert_card(&mut self, i: usize, card: Card) -> u32 {
        todo!()
    }

    /// Remove the card at `i`. O(num_cards). Its slot never reappears.
    pub fn remove_card(&mut self, i: usize) {
        todo!()
    }

    /// Lex a single snippet into a detached `Card` (used by `insert_card`).
    /// O(snippet length) — microseconds.
    pub fn lex_card(&self, text: &str, kind: CardKind) -> Card {
        todo!()
    }

    // --- in-card token override overlay -------------------------------------

    /// Overwrite one token's text in place, promoting the card to `Owned` on
    /// first touch. This is how renumbering and value edits work: local to the
    /// card, so every *other* card keeps its verbatim-copy emit fast path.
    pub fn set_token_text(&mut self, card: usize, token: usize, text: impl Into<String>) {
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
    pub fn parse(src: impl Into<Arc<str>>) -> Model {
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
