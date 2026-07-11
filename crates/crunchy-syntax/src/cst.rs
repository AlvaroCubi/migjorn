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
use std::collections::HashMap;

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
    /// Sparse per-token text overrides (empty until the first edit).
    overrides: HashMap<u32, Box<str>>,
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
            Some(o) => Cow::Borrowed(o),
            None => Cow::Borrowed(self.token_src_text(i)),
        }
    }

    /// Byte span of token `i` in the original source.
    #[inline]
    pub fn token_span(&self, i: u32) -> Span {
        Span::new(self.tok_start[i as usize], self.tok_start[i as usize + 1])
    }

    /// Override the text of token `i`. Used by higher layers to implement
    /// semantic edits (e.g. renumbering) losslessly.
    pub fn set_token_text(&mut self, i: u32, text: impl Into<Box<str>>) {
        self.overrides.insert(i, text.into());
    }

    /// Whether any edits have been applied.
    #[inline]
    pub fn is_edited(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Iterate the *meaningful* (non-trivia) token indices of a card.
    pub fn card_content_tokens<'a>(&'a self, card: &Card) -> impl Iterator<Item = u32> + 'a {
        (card.first_tok..card.tok_end).filter(move |&i| !self.token_kind(i).is_trivia())
    }

    /// Re-emit the tree as source text, applying any overrides. Byte-for-byte
    /// identical to the input when unedited.
    pub fn to_source(&self) -> String {
        // Capacity: source length is exact when unedited and a good estimate
        // otherwise.
        let mut out = String::with_capacity(self.src.len());
        let n = self.tok_kind.len();
        if self.overrides.is_empty() {
            out.push_str(&self.src);
            return out;
        }
        for i in 0..n {
            match self.overrides.get(&(i as u32)) {
                Some(o) => out.push_str(o),
                None => {
                    let s = self.tok_start[i] as usize;
                    let e = self.tok_start[i + 1] as usize;
                    out.push_str(&self.src[s..e]);
                }
            }
        }
        out
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
    let cards = build_cards(&lines, &tok_kind, &mut diagnostics, src.as_bytes(), &tok_start);

    let tree = GreenTree {
        src,
        tok_kind,
        tok_start,
        cards,
        overrides: HashMap::new(),
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
    src: &[u8],
    tok_start: &[u32],
) -> Vec<Card> {
    let n_tok = tok_kind.len() as u32;
    let mut cards: Vec<Card> = Vec::new();
    if lines.is_empty() {
        return cards;
    }

    // Title is line 0 (message blocks are not yet handled; see M1 notes). It is
    // always the title even if it lexes as a comment (e.g. a deck whose title
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
    if std::env::var_os("CRUNCHY_DEBUG").is_some() {
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
                let is_continuation =
                    cur.is_some() && (l.leading_cols >= 5 || prev_content_amp);
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

    // Suppress unused-parameter lint in builds where `src` is not otherwise
    // referenced here (kept for future column-based diagnostics).
    let _ = src;
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(p: &Parsed) -> Vec<SyntaxKind> {
        p.tree.cards().iter().map(|c| c.kind).collect()
    }

    #[test]
    fn roundtrips_full_deck() {
        let src = "\u{feff}Test title\n1 0 -1 imp:n=1 $ void\n2 0 1\n\n1 PX 0.0\n2 SO 5\n\nm1 1001 1\n";
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
        assert_eq!(p.tree.to_source(), "title\n999 0 -1\n\n1 PX 0\n\nm1 1001 1\n");
    }

    #[test]
    fn missing_delimiters_warn_but_still_parse() {
        let src = "title\n1 0 -1\n2 PX 0\n";
        let p = parse(src);
        assert!(!p.diagnostics.is_empty());
        assert_eq!(p.tree.to_source(), src);
    }
}
