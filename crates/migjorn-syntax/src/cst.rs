//! The lossless concrete syntax tree (CST) and the parser that builds it.
//!
//! # Representation
//!
//! Everything is stored in flat, contiguous arrays over the original source
//! (the M0 finding: this is ~7x faster than an interned rowan tree and stays
//! byte-for-byte lossless):
//!
//! - **Tokens** tile the source with no gaps. Token `i` covers
//!   `src[tok_start[i] .. tok_start[i + 1]]`; `tok_start` has a trailing
//!   sentinel equal to `src.len()`. Token text is therefore a *span*, never a
//!   copy.
//! - **Cards** are the logical lines (a starting line plus its continuation and
//!   trailing-trivia lines). Each card owns a contiguous token range, and the
//!   cards tile the whole token stream. This two-level tree (root → cards →
//!   tokens) is all the structure M1 needs; sub-card structure (geometry
//!   expressions, coefficient lists) is layered on in later milestones.
//!
//! # Edits
//!
//! A token's text can be *overridden* without touching the source. Re-emission
//! prefers the override when present. This makes bulk semantic edits — most
//! importantly **whole-geometry renumbering**, which rewrites every cell/surface
//! id and every reference to it — an `O(edited tokens)` operation that leaves all
//! untouched bytes exactly as they were.

use std::borrow::Cow;
use std::fmt::Write as _;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::diagnostics::{Diagnostic, Span};
use crate::lexer::lex;
use crate::SyntaxKind;

/// A card node: a kind plus the contiguous token range `[first_tok, tok_end)`.
#[derive(Debug, Clone, Copy)]
pub struct Card {
    pub kind: SyntaxKind,
    pub first_tok: u32,
    pub tok_end: u32,
}

/// A lossless syntax tree over an owned source string.
pub struct GreenTree {
    src: String,
    /// Token kinds as raw `u16` discriminants (SoA for cache-friendly scans).
    tok_kind: Vec<u16>,
    /// Token start offsets; `tok_start.len() == tok_kind.len() + 1`, with a
    /// trailing sentinel of `src.len()`.
    tok_start: Vec<u32>,
    /// Card nodes in source order; they tile the token stream.
    cards: Vec<Card>,
    /// Stable identity for each card position: `card_slots[pos]` is a slot id
    /// that is never reused for the life of the tree. Handles key on the slot,
    /// so inserting/deleting/reordering cards never invalidates a handle that
    /// still refers to a live card.
    card_slots: Vec<u32>,
    /// Reverse map `slot -> position`. Rebuilt whenever the card list changes
    /// (structural edits); O(1) lookup for handle resolution.
    slot_to_pos: FxHashMap<u32, usize>,
    /// Next slot id to hand out (monotonic; never reused).
    next_slot: u32,
    /// Sparse per-token overrides (empty until the first edit). A fast
    /// (non-DoS-resistant) hasher — keys are our own token indices, and bulk
    /// renumbering does tens of millions of inserts + lookups.
    overrides: FxHashMap<u32, Override>,
    /// Structurally-edited cards: `slot -> replacement content text`. On
    /// re-emission the card's meaningful token span is replaced by this text,
    /// while surrounding trivia is preserved (see `replace_card_content`).
    card_replacements: FxHashMap<u32, Box<str>>,
    /// Sparse token *insertions* keyed by a "gap" index: the text in
    /// `insertions[g]` is emitted immediately **before** token `g` (a gap equal
    /// to `token_count` means "at end of stream"). A `Vec` preserves the order
    /// of multiple inserts at the same gap. This is the token-splice complement
    /// to `overrides` (which can only rewrite an existing token in place): it
    /// lets an edit add tokens while every untouched byte stays a source span.
    insertions: FxHashMap<u32, Vec<Box<str>>>,
    /// Token indices skipped entirely on re-emission (the deletion half of the
    /// splice overlay). Empty until the first structural edit.
    deletions: FxHashSet<u32>,
}

/// A replacement value for a token. `Int` avoids allocating a string per edit,
/// which matters for bulk edits like whole-geometry renumbering (millions of
/// numeric tokens); the digits are written directly on re-emission.
#[derive(Debug, Clone)]
enum Override {
    Int(i64),
    Text(Box<str>),
}

impl GreenTree {
    /// Number of tokens.
    #[inline]
    pub fn token_count(&self) -> usize {
        self.tok_kind.len()
    }

    /// The card nodes, in source order.
    #[inline]
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// The stable slot id of the card at position `pos`.
    #[inline]
    pub fn card_slot(&self, pos: usize) -> u32 {
        self.card_slots[pos]
    }

    /// The current position of the card with stable slot `slot`, or `None` if
    /// no live card has that slot (e.g. it was deleted).
    #[inline]
    pub fn card_by_slot(&self, slot: u32) -> Option<usize> {
        self.slot_to_pos.get(&slot).copied()
    }

    /// The next slot id that would be assigned to a newly inserted card. Slots
    /// are monotonic and never reused, so a value observed here is a lower
    /// bound on the ids of any cards inserted afterward.
    #[inline]
    pub fn next_slot(&self) -> u32 {
        self.next_slot
    }

    /// Reassign the stable card slots (position → slot) and the monotonic
    /// counter. Used after a structural splice-and-reparse to preserve handle
    /// identity: cards that carried over keep their old slot, and any new card
    /// gets a fresh one. `slots.len()` must equal the number of cards.
    pub fn set_card_slots(&mut self, slots: Vec<u32>, next_slot: u32) {
        assert_eq!(
            slots.len(),
            self.cards.len(),
            "set_card_slots: one slot per card"
        );
        self.slot_to_pos = slots.iter().enumerate().map(|(pos, &s)| (s, pos)).collect();
        self.card_slots = slots;
        self.next_slot = next_slot;
    }

    /// The original source.
    #[inline]
    pub fn src(&self) -> &str {
        &self.src
    }

    /// Kind of token `i`.
    #[inline]
    pub fn token_kind(&self, i: u32) -> SyntaxKind {
        SyntaxKind::from_u16(self.tok_kind[i as usize])
    }

    /// The *original* source text of token `i`, ignoring any override.
    #[inline]
    pub fn token_src_text(&self, i: u32) -> &str {
        let s = self.tok_start[i as usize] as usize;
        let e = self.tok_start[i as usize + 1] as usize;
        &self.src[s..e]
    }

    /// The *effective* text of token `i` (override if set, else source).
    #[inline]
    pub fn token_text(&self, i: u32) -> Cow<'_, str> {
        match self.overrides.get(&i) {
            Some(Override::Text(o)) => Cow::Borrowed(o),
            Some(Override::Int(v)) => Cow::Owned(v.to_string()),
            None => Cow::Borrowed(self.token_src_text(i)),
        }
    }

    /// Byte span of token `i` in the original source.
    #[inline]
    pub fn token_span(&self, i: u32) -> Span {
        Span::new(self.tok_start[i as usize], self.tok_start[i as usize + 1])
    }

    /// Override the text of token `i`. Used by higher layers to implement
    /// semantic edits losslessly.
    pub fn set_token_text(&mut self, i: u32, text: impl Into<Box<str>>) {
        self.overrides.insert(i, Override::Text(text.into()));
    }

    /// Override token `i` with an integer value, formatted on re-emission.
    /// Allocation-free per edit — the path for bulk renumbering.
    pub fn set_token_int(&mut self, i: u32, value: i64) {
        self.overrides.insert(i, Override::Int(value));
    }

    /// Splice `text` into the stream so it is emitted immediately **before**
    /// token `i` (i.e. at the gap on `i`'s left). Multiple inserts at the same
    /// gap are emitted in call order. The caller owns separator whitespace: the
    /// text is emitted verbatim, so include a leading/trailing space where MCNP
    /// needs one. Every other byte stays a source span.
    pub fn insert_before(&mut self, i: u32, text: impl Into<Box<str>>) {
        self.insertions.entry(i).or_default().push(text.into());
    }

    /// Splice `text` in so it is emitted immediately **after** token `i` (the
    /// gap on `i`'s right, i.e. before token `i + 1`). See [`insert_before`] for
    /// separator ownership.
    ///
    /// [`insert_before`]: Self::insert_before
    pub fn insert_after(&mut self, i: u32, text: impl Into<Box<str>>) {
        self.insertions.entry(i + 1).or_default().push(text.into());
    }

    /// Replace *all* insertions at the gap before token `i` with a single
    /// `text`. Used when a value spliced in earlier (e.g. a void→real density
    /// placeholder) is later revised, so the edit stays idempotent.
    pub fn set_insertion_before(&mut self, i: u32, text: impl Into<Box<str>>) {
        self.insertions.insert(i, vec![text.into()]);
    }

    /// Drop any insertions at the gap before token `i` (undo an `insert_before`).
    pub fn clear_insertion_before(&mut self, i: u32) {
        self.insertions.remove(&i);
    }

    /// Delete token `i`: it is skipped entirely on re-emission. Trivia around it
    /// is preserved, so callers removing a meaningful token usually delete one
    /// adjacent whitespace token too (to avoid a doubled or dangling separator).
    pub fn delete_token(&mut self, i: u32) {
        self.deletions.insert(i);
    }

    /// Un-delete token `i` (undo a `delete_token`), restoring it on re-emission.
    pub fn undelete_token(&mut self, i: u32) {
        self.deletions.remove(&i);
    }

    /// Whether any edits have been applied.
    #[inline]
    pub fn is_edited(&self) -> bool {
        !self.overrides.is_empty()
            || !self.card_replacements.is_empty()
            || !self.insertions.is_empty()
            || !self.deletions.is_empty()
    }

    /// Whether any *splice* edits (insertions, deletions, or whole-card
    /// replacements) are pending — the emit-only overlay that a token-based
    /// reader cannot see. Deliberately **excludes** token overrides, which
    /// `token_text`/emission already apply consistently: a caller re-reading
    /// tokens after an override sees the effective value, so overrides alone
    /// never require materialising the tree.
    #[inline]
    pub fn has_pending_splices(&self) -> bool {
        !self.card_replacements.is_empty()
            || !self.insertions.is_empty()
            || !self.deletions.is_empty()
    }

    /// Iterate the *meaningful* (non-trivia) token indices of a card.
    pub fn card_content_tokens<'a>(&'a self, card: &Card) -> impl Iterator<Item = u32> + 'a {
        (card.first_tok..card.tok_end).filter(move |&i| !self.token_kind(i).is_trivia())
    }

    /// Re-emit the tree as source text, applying any overrides and card
    /// replacements. Byte-for-byte identical to the input when unedited.
    pub fn to_source(&self) -> String {
        // Capacity: source length is exact when unedited and a good estimate
        // otherwise.
        let mut out = String::with_capacity(self.src.len());
        if self.overrides.is_empty()
            && self.card_replacements.is_empty()
            && self.insertions.is_empty()
            && self.deletions.is_empty()
        {
            out.push_str(&self.src);
            return out;
        }
        let ranges = self.replaced_ranges();
        self.emit_range(0, self.tok_kind.len() as u32, &ranges, &mut out);
        out
    }

    /// The exact source text of the card at `card_index`, applying any overrides
    /// and card replacement. This is the card's whole token span, so it includes
    /// inline `$` comments, absorbed comment lines, and `&`/indent continuations
    /// — which is what "the text of this card" means for exploration
    /// (`"$ vacuum vessel" in cell.text`). Reflects edits made so far.
    pub fn card_source(&self, card_index: usize) -> String {
        let card = self.cards[card_index];
        let ranges = self.replaced_ranges();
        let mut out = String::new();
        self.emit_range(card.first_tok, card.tok_end, &ranges, &mut out);
        out
    }

    /// The exact source text of the half-open token range `[from, to)`,
    /// applying any overrides and card replacements — the primitive
    /// `card_source` is built on, generalized to a sub-range. Used to split a
    /// card's own content from a trailing comment block absorbed onto it.
    pub fn token_range_source(&self, from: u32, to: u32) -> String {
        let ranges = self.replaced_ranges();
        let mut out = String::new();
        self.emit_range(from, to, &ranges, &mut out);
        out
    }

    /// Token index where the card at `card_index` ends its own content and a
    /// trailing block of whole `c`/`$` comment lines begins (or `card.tok_end`
    /// if it absorbed none). A card only ever absorbs comments onto its
    /// *tail* (see `build_cards`), so everything before this index — content
    /// lines, `&`/indent continuations, and any inline `$` comment on the
    /// last content line — is the card's own text; everything from here on is
    /// a header an author wrote for whatever card follows, misattached
    /// backward onto this one.
    pub fn card_header_split(&self, card_index: usize) -> u32 {
        let card = self.cards[card_index];
        let mut i = match self.card_content_tokens(&card).last() {
            Some(last) => {
                let mut j = last + 1;
                while j < card.tok_end
                    && matches!(
                        self.token_kind(j),
                        SyntaxKind::WHITESPACE | SyntaxKind::DOLLAR_COMMENT
                    )
                {
                    j += 1;
                }
                j
            }
            // No content token at all: only possible for a comment-shaped
            // title (e.g. a model whose title is literally "C") — its whole
            // first line is the title, not a header for what follows.
            None => {
                let mut j = card.first_tok;
                while j < card.tok_end && self.token_kind(j) != SyntaxKind::NEWLINE {
                    j += 1;
                }
                j
            }
        };
        if i < card.tok_end && self.token_kind(i) == SyntaxKind::NEWLINE {
            i += 1;
        }
        i
    }

    /// Replace the *content* of the card at `card_index` with `text` on
    /// re-emission. Only the card's meaningful (non-trivia) token span is
    /// replaced; leading indentation and trailing trivia — newlines, the block's
    /// blank delimiter, inline `$` comments, absorbed comment lines — are
    /// preserved, so the structure around the card stays intact. Keyed by the
    /// card's stable slot, so it survives later structural edits.
    pub fn replace_card_content(&mut self, card_index: usize, text: impl Into<Box<str>>) {
        let slot = self.card_slots[card_index];
        self.card_replacements.insert(slot, text.into());
    }

    /// Whether the card at `card_index` has a whole-content replacement (the
    /// lossy fallback emit). A card in this mode must not also carry token
    /// splices for the same content, or they would double-apply; callers switch
    /// such a card fully into replace mode via [`clear_card_overlay`].
    ///
    /// [`clear_card_overlay`]: Self::clear_card_overlay
    pub fn card_has_replacement(&self, card_index: usize) -> bool {
        self.card_replacements
            .contains_key(&self.card_slots[card_index])
    }

    /// Drop every token override and splice (insertion/deletion) anchored within
    /// the card at `card_index`. Used before a whole-card replacement so the two
    /// emit mechanisms never overlap on one card.
    pub fn clear_card_overlay(&mut self, card_index: usize) {
        let card = self.cards[card_index];
        for i in card.first_tok..card.tok_end {
            self.overrides.remove(&i);
            self.insertions.remove(&i);
            self.deletions.remove(&i);
        }
    }

    /// Effective source text of a cell's parameter region — the tokens from
    /// `params_start` through `last_content`, with overrides and splices applied,
    /// **plus** any parameters spliced in immediately after the last content
    /// token. Used when promoting or re-emitting a cell so splice-added/removed
    /// parameters are reflected. Returns an empty string when there is no
    /// parameter section and none was spliced in.
    pub fn params_effective_text(&self, params_start: Option<u32>, last_content: u32) -> String {
        let ranges = self.replaced_ranges();
        let mut out = String::new();
        if let Some(ps) = params_start {
            self.emit_range(ps, last_content + 1, &ranges, &mut out);
        }
        // Parameters appended after the last content token live at this gap.
        self.emit_insertions(last_content + 1, &mut out);
        if params_start.is_none() {
            // A param spliced onto a previously param-less cell carries a leading
            // separator; drop it so the tail matches `params_text` conventions.
            return out.trim_start().to_string();
        }
        out
    }

    /// First and last meaningful (non-trivia) token indices of a card, if any.
    fn content_bounds(&self, card: &Card) -> Option<(u32, u32)> {
        let mut first = None;
        let mut last = None;
        for i in self.card_content_tokens(card) {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
        Some((first?, last?))
    }

    /// Sorted, non-overlapping `(content_first, content_last, replacement)` for
    /// every live replaced card. Cheap: there are as many entries as cards that
    /// have actually been restructured.
    fn replaced_ranges(&self) -> Vec<(u32, u32, &str)> {
        let mut v: Vec<(u32, u32, &str)> = self
            .card_replacements
            .iter()
            .filter_map(|(&slot, text)| {
                let pos = self.card_by_slot(slot)?;
                let (f, l) = self.content_bounds(&self.cards[pos])?;
                Some((f, l, text.as_ref()))
            })
            .collect();
        v.sort_by_key(|r| r.0);
        v
    }

    /// Emit tokens `[from, to)`, applying overrides, splice insertions/deletions,
    /// and substituting the emitted text for any replaced card whose content
    /// range begins within. `ranges` must be sorted by start and non-overlapping
    /// (see `replaced_ranges`). A card is either replaced or spliced, never both.
    fn emit_range(&self, from: u32, to: u32, ranges: &[(u32, u32, &str)], out: &mut String) {
        let mut ri = ranges.partition_point(|r| r.0 < from);
        let mut i = from;
        while i < to {
            self.emit_insertions(i, out);
            if ri < ranges.len() && ranges[ri].0 == i {
                out.push_str(ranges[ri].2);
                i = ranges[ri].1 + 1;
                ri += 1;
            } else {
                if !self.deletions.contains(&i) {
                    self.emit_token(i, out);
                }
                i += 1;
            }
        }
        // Insertions at the very end of the stream (gap == token_count) have no
        // following token to hang before; emit them only for a whole-tree walk,
        // not a sub-range (a card's trailing gap belongs to the next card).
        if to as usize == self.tok_kind.len() {
            self.emit_insertions(to, out);
        }
    }

    /// Emit any splice insertions anchored at gap `g` (before token `g`).
    #[inline]
    fn emit_insertions(&self, g: u32, out: &mut String) {
        if let Some(texts) = self.insertions.get(&g) {
            for t in texts {
                out.push_str(t);
            }
        }
    }

    /// Append the effective text of token `i` (override if set, else source) to
    /// `out`, without a temporary allocation for integer overrides.
    #[inline]
    fn emit_token(&self, i: u32, out: &mut String) {
        match self.overrides.get(&i) {
            Some(Override::Text(o)) => out.push_str(o),
            Some(Override::Int(v)) => {
                let _ = write!(out, "{v}");
            }
            None => out.push_str(self.token_src_text(i)),
        }
    }
}

/// Result of parsing: the tree plus any diagnostics.
pub struct Parsed {
    pub tree: GreenTree,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse MCNP source into a lossless tree with diagnostics. Never panics on
/// malformed input.
pub fn parse(src: impl Into<String>) -> Parsed {
    let src = src.into();

    // ---- 1. Lex into flat token arrays. ----
    let approx = src.len() / 4 + 16;
    let mut tok_kind: Vec<u16> = Vec::with_capacity(approx);
    let mut tok_start: Vec<u32> = Vec::with_capacity(approx + 1);
    lex(src.as_bytes(), |kind, start, _len| {
        tok_kind.push(kind as u16);
        tok_start.push(start);
    });
    tok_start.push(src.len() as u32); // sentinel

    // ---- 2. Split into physical lines. ----
    let lines = split_lines(&tok_kind, &tok_start, src.as_bytes());

    // ---- 3. Assign a section to each line and group into cards. ----
    let mut diagnostics = Vec::new();
    let cards = build_cards(&lines, &tok_kind, &mut diagnostics, &tok_start);

    // Stable card identity: at parse time slot == position. Structural edits
    // (later milestones) hand out fresh slots from `next_slot` and rebuild the
    // reverse map, so existing handles keep resolving.
    let card_slots: Vec<u32> = (0..cards.len() as u32).collect();
    let slot_to_pos: FxHashMap<u32, usize> = card_slots.iter().map(|&s| (s, s as usize)).collect();
    let next_slot = cards.len() as u32;

    let tree = GreenTree {
        src,
        tok_kind,
        tok_start,
        cards,
        card_slots,
        slot_to_pos,
        next_slot,
        overrides: FxHashMap::default(),
        card_replacements: FxHashMap::default(),
        insertions: FxHashMap::default(),
        deletions: FxHashSet::default(),
    };
    Parsed { tree, diagnostics }
}

/// Classification of a physical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineClass {
    /// Only whitespace (a block delimiter).
    Blank,
    /// A `c`/`C` comment card, no meaningful content.
    Comment,
    /// Has at least one meaningful token.
    Content,
}

struct LineInfo {
    first_tok: u32,
    tok_end: u32,
    class: LineClass,
    /// Number of leading blank columns (tab-expanded) before the first token.
    leading_cols: u32,
    /// The last meaningful token of the line is `&`.
    ends_with_amp: bool,
}

fn split_lines(tok_kind: &[u16], tok_start: &[u32], src: &[u8]) -> Vec<LineInfo> {
    let n = tok_kind.len();
    let mut lines = Vec::with_capacity(src.len() / 32 + 16);
    let mut i = 0usize;
    while i < n {
        let first = i;
        let mut has_content = false;
        let mut has_comment = false;
        let mut last_meaningful: Option<SyntaxKind> = None;
        while i < n {
            let k = SyntaxKind::from_u16(tok_kind[i]);
            match k {
                SyntaxKind::NEWLINE => {
                    i += 1;
                    break;
                }
                // A `c` card or an inline `$` comment both make the line a
                // comment line. Crucially, a whitespace + `$…` line is NOT a
                // blank delimiter — it has non-blank characters.
                SyntaxKind::COMMENT_LINE | SyntaxKind::DOLLAR_COMMENT => has_comment = true,
                k if k.is_trivia() => {}
                k => {
                    has_content = true;
                    last_meaningful = Some(k);
                }
            }
            i += 1;
        }
        let class = if has_content {
            LineClass::Content
        } else if has_comment {
            LineClass::Comment
        } else {
            LineClass::Blank
        };
        let leading_cols = leading_columns(src, tok_start[first] as usize);
        lines.push(LineInfo {
            first_tok: first as u32,
            tok_end: i as u32,
            class,
            leading_cols,
            ends_with_amp: last_meaningful == Some(SyntaxKind::AMP),
        });
    }
    lines
}

/// Count leading blank columns from byte offset `start`, expanding tabs to the
/// next multiple-of-8 column (MCNP tab-stop convention).
fn leading_columns(src: &[u8], start: usize) -> u32 {
    let mut col = 0u32;
    let mut i = start;
    while i < src.len() {
        match src[i] {
            b' ' => col += 1,
            b'\t' => col = (col / 8 + 1) * 8,
            0xEF if src.get(i + 1) == Some(&0xBB) && src.get(i + 2) == Some(&0xBF) => {
                i += 2; // BOM occupies no column
            }
            _ => break,
        }
        i += 1;
    }
    col
}

/// Section a line belongs to. Title is handled separately.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Title,
    Cell,
    Surface,
    Data,
}

fn section_card_kind(sec: Section) -> SyntaxKind {
    match sec {
        Section::Title => SyntaxKind::TITLE_CARD,
        Section::Cell => SyntaxKind::CELL_CARD,
        Section::Surface => SyntaxKind::SURFACE_CARD,
        Section::Data => SyntaxKind::DATA_CARD,
    }
}

fn build_cards(
    lines: &[LineInfo],
    tok_kind: &[u16],
    diagnostics: &mut Vec<Diagnostic>,
    tok_start: &[u32],
) -> Vec<Card> {
    let n_tok = tok_kind.len() as u32;
    let mut cards: Vec<Card> = Vec::new();
    if lines.is_empty() {
        return cards;
    }

    // Title is line 0 (message blocks are not yet handled; see M1 notes). It is
    // always the title even if it lexes as a comment (e.g. a model whose title
    // is literally "C").
    let title_line = 0usize;

    // Find the first two blank delimiters after the title -> block boundaries.
    let mut blank1 = None;
    let mut blank2 = None;
    for (idx, l) in lines.iter().enumerate().skip(title_line + 1) {
        if l.class == LineClass::Blank {
            if blank1.is_none() {
                blank1 = Some(idx);
            } else if blank2.is_none() {
                blank2 = Some(idx);
                break;
            }
        }
    }
    if std::env::var_os("MIGJORN_DEBUG").is_some() {
        eprintln!(
            "[debug] lines={} blank1={:?} blank2={:?}",
            lines.len(),
            blank1,
            blank2
        );
        for &b in [blank1, blank2].iter().flatten() {
            let byte = tok_start[lines[b].first_tok as usize];
            eprintln!("[debug]   blank line idx {b} at byte {byte}");
        }
    }
    if blank1.is_none() || blank2.is_none() {
        let end = *tok_start.last().unwrap();
        diagnostics.push(Diagnostic::warning(
            Span::new(0, end.min(1)),
            format!(
                "expected two blank-line delimiters (cell|surface|data); found {}",
                blank1.is_some() as u8 + blank2.is_some() as u8
            ),
        ));
    }

    let section_of = |idx: usize| -> Section {
        if idx == title_line {
            Section::Title
        } else if blank1.is_none_or(|b| idx < b) {
            Section::Cell
        } else if blank2.is_none_or(|b| idx < b) {
            Section::Surface
        } else {
            Section::Data
        }
    };

    // Walk lines, grouping into cards. Cards tile the token stream: each card's
    // range runs from its start up to the start of the next card.
    let mut cur: Option<(SyntaxKind, u32)> = None; // (kind, first_tok)
    let mut prev_content_amp = false;

    let close = |cards: &mut Vec<Card>, cur: &mut Option<(SyntaxKind, u32)>, end: u32| {
        if let Some((kind, first)) = cur.take() {
            cards.push(Card {
                kind,
                first_tok: first,
                tok_end: end,
            });
        }
    };

    for (idx, l) in lines.iter().enumerate() {
        let is_title = idx == title_line;

        if is_title {
            // Start the title card (nothing precedes it but leading trivia/BOM,
            // which is absorbed because first_tok is this line's first token).
            close(&mut cards, &mut cur, l.first_tok);
            cur = Some((SyntaxKind::TITLE_CARD, l.first_tok));
            prev_content_amp = l.ends_with_amp;
            continue;
        }

        match l.class {
            LineClass::Blank => {
                // Delimiter: close the current card, absorbing through this
                // blank line, and reset so following trivia attaches to the
                // next block's first card.
                close(&mut cards, &mut cur, l.tok_end);
                prev_content_amp = false;
            }
            LineClass::Comment => {
                // Trivia: absorbed into the current card if one is open, else
                // left pending to attach to the next card. Do not reset amp.
            }
            LineClass::Content => {
                let is_continuation = cur.is_some() && (l.leading_cols >= 5 || prev_content_amp);
                if !is_continuation {
                    // New card starts here; the previous card absorbs any
                    // trailing trivia up to this line's first token.
                    close(&mut cards, &mut cur, l.first_tok);
                    cur = Some((section_card_kind(section_of(idx)), l.first_tok));
                }
                prev_content_amp = l.ends_with_amp;
            }
        }
    }
    close(&mut cards, &mut cur, n_tok);
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(p: &Parsed) -> Vec<SyntaxKind> {
        p.tree.cards().iter().map(|c| c.kind).collect()
    }

    #[test]
    fn roundtrips_full_model() {
        let src =
            "\u{feff}Test title\n1 0 -1 imp:n=1 $ void\n2 0 1\n\n1 PX 0.0\n2 SO 5\n\nm1 1001 1\n";
        let p = parse(src);
        assert_eq!(p.tree.to_source(), src);
    }

    #[test]
    fn splits_blocks_into_cards() {
        let src = "title\n1 0 -1\n2 0 1\n\n10 PX 0\n11 PY 0\n\nsdef pos=0 0 0\n";
        let p = parse(src);
        assert_eq!(
            kinds(&p),
            vec![
                SyntaxKind::TITLE_CARD,
                SyntaxKind::CELL_CARD,
                SyntaxKind::CELL_CARD,
                SyntaxKind::SURFACE_CARD,
                SyntaxKind::SURFACE_CARD,
                SyntaxKind::DATA_CARD,
            ]
        );
    }

    #[test]
    fn title_that_looks_like_a_comment_is_the_title() {
        // First line "C" is the title, not a comment card.
        let src = "C\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\n";
        let p = parse(src);
        assert_eq!(p.tree.cards()[0].kind, SyntaxKind::TITLE_CARD);
    }

    #[test]
    fn continuation_by_indent_joins_card() {
        // Second physical line is indented >=5 cols -> same card.
        let src = "title\n1 0 -1 2 -3\n     4 -5\n\n1 PX 0\n\nm1 1001 1\n";
        let p = parse(src);
        let cell_cards: Vec<_> = p
            .tree
            .cards()
            .iter()
            .filter(|c| c.kind == SyntaxKind::CELL_CARD)
            .collect();
        assert_eq!(cell_cards.len(), 1, "indented continuation must join");
    }

    #[test]
    fn continuation_by_ampersand_joins_card() {
        let src = "title\n1 0 -1 &\n2 -3\n\n1 PX 0\n\nm1 1001 1\n";
        let p = parse(src);
        let cell_cards = p
            .tree
            .cards()
            .iter()
            .filter(|c| c.kind == SyntaxKind::CELL_CARD)
            .count();
        assert_eq!(cell_cards, 1, "ampersand continuation must join");
    }

    #[test]
    fn comment_lines_do_not_break_cards() {
        let src = "title\n1 0 -1\nc a comment\n2 0 1\n\n1 PX 0\n\nm1 1001 1\n";
        let p = parse(src);
        let cells = p
            .tree
            .cards()
            .iter()
            .filter(|c| c.kind == SyntaxKind::CELL_CARD)
            .count();
        assert_eq!(cells, 2);
        assert_eq!(p.tree.to_source(), src);
    }

    #[test]
    fn token_override_roundtrips_the_edit() {
        // Renumber the first cell's id from 1 to 999 by overriding its token.
        let src = "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        // Find the first NUMBER token of the first cell card.
        let cell = *p
            .tree
            .cards()
            .iter()
            .find(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap();
        let id_tok = p
            .tree
            .card_content_tokens(&cell)
            .next()
            .expect("cell has an id token");
        assert_eq!(p.tree.token_text(id_tok), "1");
        p.tree.set_token_text(id_tok, "999");
        assert_eq!(
            p.tree.to_source(),
            "title\n999 0 -1\n\n1 PX 0\n\nm1 1001 1\n"
        );
    }

    #[test]
    fn card_source_includes_inline_comment() {
        let src = "title\n1 0 -1 imp:n=1 $ vacuum vessel\n\n1 PX 0\n\nm1 1001 1\n";
        let p = parse(src);
        // Card 1 is the first cell card.
        let cell_pos = p
            .tree
            .cards()
            .iter()
            .position(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap();
        let text = p.tree.card_source(cell_pos);
        assert!(text.contains("$ vacuum vessel"), "got: {text:?}");
        assert!(text.contains("1 0 -1"));
    }

    #[test]
    fn card_source_reflects_overrides() {
        let src = "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell_pos = p
            .tree
            .cards()
            .iter()
            .position(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap();
        let id_tok = p.tree.cards()[cell_pos].first_tok;
        p.tree.set_token_int(id_tok, 42);
        assert!(p.tree.card_source(cell_pos).contains("42 0 -1"));
    }

    #[test]
    fn replace_card_content_preserves_surrounding_structure() {
        let src = "title\n1 0 -1 imp:n=1 $ fuel\n2 0 1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell1 = p
            .tree
            .cards()
            .iter()
            .position(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap();
        p.tree.replace_card_content(cell1, "1 5 -2.0 -1");
        let out = p.tree.to_source();
        // Content replaced; trailing inline comment and the rest are preserved.
        assert!(out.contains("1 5 -2.0 -1 $ fuel"), "got: {out}");
        assert!(out.contains("2 0 1"));
        assert!(out.contains("\n\n1 PX 0"), "block delimiter lost: {out}");
    }

    #[test]
    fn replace_last_card_in_block_keeps_delimiter() {
        // The last cell before the blank line absorbs the delimiter; replacing
        // its content must not swallow that blank line.
        let src = "title\n1 0 -1\n2 0 1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let last_cell = p
            .tree
            .cards()
            .iter()
            .enumerate()
            .rfind(|(_, c)| c.kind == SyntaxKind::CELL_CARD)
            .unwrap()
            .0;
        p.tree.replace_card_content(last_cell, "2 0 1 -3");
        let out = p.tree.to_source();
        assert!(out.contains("2 0 1 -3\n\n1 PX 0"), "delimiter lost: {out}");
        // Re-parse stays well-formed (three cell/surface blocks intact).
        assert!(parse(&out).diagnostics.is_empty());
    }

    #[test]
    fn card_source_reflects_replacement() {
        let src = "title\n1 0 -1 imp:n=1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell1 = p
            .tree
            .cards()
            .iter()
            .position(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap();
        p.tree.replace_card_content(cell1, "1 0 -1 5");
        assert!(p.tree.card_source(cell1).contains("1 0 -1 5"));
    }

    #[test]
    fn slots_resolve_positions() {
        let src = "title\n1 0 -1\n2 0 1\n\n1 PX 0\n\nm1 1001 1\n";
        let p = parse(src);
        for pos in 0..p.tree.cards().len() {
            let slot = p.tree.card_slot(pos);
            assert_eq!(p.tree.card_by_slot(slot), Some(pos));
        }
        assert_eq!(p.tree.next_slot(), p.tree.cards().len() as u32);
    }

    #[test]
    fn missing_delimiters_warn_but_still_parse() {
        let src = "title\n1 0 -1\n2 PX 0\n";
        let p = parse(src);
        assert!(!p.diagnostics.is_empty());
        assert_eq!(p.tree.to_source(), src);
    }

    /// Find the first token in `card` whose effective text equals `text`.
    fn tok_with_text(tree: &GreenTree, card: &Card, text: &str) -> u32 {
        (card.first_tok..card.tok_end)
            .find(|&i| tree.token_text(i) == text)
            .unwrap_or_else(|| panic!("no token {text:?} in card"))
    }

    fn first_cell(p: &Parsed) -> Card {
        *p.tree
            .cards()
            .iter()
            .find(|c| c.kind == SyntaxKind::CELL_CARD)
            .unwrap()
    }

    #[test]
    fn insert_after_splices_before_following_trivia() {
        // Adding a surface lands after the last geometry token and before the
        // newline, so anything on the continuation line stays put.
        let src = "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell = first_cell(&p);
        let neg1 = tok_with_text(&p.tree, &cell, "-1");
        p.tree.insert_after(neg1, " -2");
        assert_eq!(
            p.tree.to_source(),
            "title\n1 0 -1 -2\n\n1 PX 0\n\nm1 1001 1\n"
        );
    }

    #[test]
    fn insert_before_splices_at_left_gap() {
        let src = "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell = first_cell(&p);
        let neg1 = tok_with_text(&p.tree, &cell, "-1");
        p.tree.insert_before(neg1, "2 ");
        assert_eq!(
            p.tree.to_source(),
            "title\n1 0 2 -1\n\n1 PX 0\n\nm1 1001 1\n"
        );
    }

    #[test]
    fn delete_token_with_separator() {
        let src = "title\n1 0 -1 2\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell = first_cell(&p);
        let two = tok_with_text(&p.tree, &cell, "2");
        p.tree.delete_token(two);
        p.tree.delete_token(two - 1); // the whitespace before it
        assert_eq!(p.tree.to_source(), "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1\n");
    }

    #[test]
    fn override_insert_delete_compose_on_one_card() {
        let src = "title\n1 0 -1 2 3\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell = first_cell(&p);
        // Renumber the id (override), drop surface 2 (delete + its ws), append -4.
        let id_tok = p.tree.card_content_tokens(&cell).next().unwrap();
        p.tree.set_token_int(id_tok, 7);
        let two = tok_with_text(&p.tree, &cell, "2");
        p.tree.delete_token(two);
        p.tree.delete_token(two - 1);
        let three = tok_with_text(&p.tree, &cell, "3");
        p.tree.insert_after(three, " -4");
        assert_eq!(
            p.tree.to_source(),
            "title\n7 0 -1 3 -4\n\n1 PX 0\n\nm1 1001 1\n"
        );
    }

    #[test]
    fn insertion_is_adjacent_when_no_surrounding_whitespace() {
        // `#(...)` tokens sit with zero whitespace between them; an insertion at
        // that gap is emitted with exactly the bytes given, no implied spacing.
        let src = "title\n1 0 #(4:2)\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cell = first_cell(&p);
        let four = tok_with_text(&p.tree, &cell, "4");
        // insert before the "4" inside the parens (a zero-whitespace gap after `(`).
        p.tree.insert_before(four, "3:");
        assert_eq!(
            p.tree.to_source(),
            "title\n1 0 #(3:4:2)\n\n1 PX 0\n\nm1 1001 1\n"
        );
    }

    #[test]
    fn insertion_at_end_of_stream() {
        // No trailing newline: the last token is the final "1"; insert_after must
        // still emit (gap == token_count).
        let src = "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1";
        let mut p = parse(src);
        let last = (p.tree.token_count() as u32) - 1;
        assert_eq!(p.tree.token_text(last), "1");
        p.tree.insert_after(last, " 2");
        assert_eq!(p.tree.to_source(), "title\n1 0 -1\n\n1 PX 0\n\nm1 1001 1 2");
    }

    #[test]
    fn card_source_excludes_next_cards_leading_insertion() {
        // An insertion anchored at the next card's first token must not leak into
        // this card's `card_source`.
        let src = "title\n1 0 -1\n2 0 1\n\n1 PX 0\n\nm1 1001 1\n";
        let mut p = parse(src);
        let cells: Vec<usize> = p
            .tree
            .cards()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == SyntaxKind::CELL_CARD)
            .map(|(i, _)| i)
            .collect();
        let second = p.tree.cards()[cells[1]];
        p.tree.insert_before(second.first_tok, "X");
        // The first cell's source ends at the gap owned by the second card.
        assert!(!p.tree.card_source(cells[0]).contains('X'));
        assert!(p.tree.card_source(cells[1]).starts_with('X'));
    }
}
