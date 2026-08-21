//! Source text -> an ordered list of self-contained cards.
//!
//! This is where the file's block structure and the two continuation mechanisms
//! are resolved. Every byte of the input ends up inside exactly one card's
//! `text`, in order, which is what makes [`crate::Cst::to_source`] a plain
//! concatenation and losslessness structural rather than reconstructed.

use rayon::prelude::*;

use crate::card::Card;
use crate::kind::CardKind;
use crate::lines::{
    ends_with_ampersand, find_blank_lines, find_card_start, is_blank, is_blank_col_continuation,
    is_comment_line, line_end, starts_with_message,
};
use std::ops::Range;

/// Below this many bytes a block is segmented on one thread; the coordination
/// overhead is not worth it for small files.
const PARALLEL_THRESHOLD: usize = 1 << 20;

/// One piece of the file, in order.
enum Segment {
    /// A blank line — its own card, and the delimiter that ends a block.
    Blank(Range<usize>),
    /// Exactly one line, taken as the title regardless of what it looks like.
    Title(Range<usize>),
    /// A run of content lines with no blank line inside it.
    Block(Range<usize>, CardKind),
}

/// Split `src` into cards. Slots are assigned by the caller.
pub(crate) fn segment(src: &str) -> Vec<Card> {
    let segments = plan(src);

    // Blocks are independent, so each one's cards can be built in parallel and
    // the results concatenated in segment order.
    let per_segment: Vec<Vec<Card>> = segments
        .iter()
        .map(|seg| match seg {
            Segment::Blank(r) => vec![Card::new(CardKind::Blank, src[r.clone()].to_owned())],
            Segment::Title(r) => vec![Card::new(CardKind::Title, src[r.clone()].to_owned())],
            Segment::Block(r, kind) => segment_block(src, r.clone(), *kind),
        })
        .collect();

    let total = per_segment.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(total);
    for cards in per_segment {
        out.extend(cards);
    }
    out
}

/// Work out the block structure. Blank lines fully determine it, because a blank
/// line ends the block it follows — so no blank line can occur *inside* a block.
fn plan(src: &str) -> Vec<Segment> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let blanks = find_blank_lines(bytes);
    let mut segments = Vec::new();
    let mut blank_idx = 0usize;
    let mut pos = 0usize;

    // Optional leading MESSAGE block, terminated by the first blank line.
    if starts_with_message(bytes) {
        let end = blanks.first().map_or(len, |b| b.0);
        if pos < end {
            segments.push(Segment::Block(pos..end, CardKind::Message));
        }
        pos = end;
        if let Some(&(bs, be)) = blanks.first() {
            segments.push(Segment::Blank(bs..be));
            pos = be;
            blank_idx = 1;
        }
    }

    // The title is positional: whatever single line sits here, even if it looks
    // like a comment. (The reference model's title is the single byte `C`.)
    if pos < len {
        let end = line_end(bytes, pos);
        segments.push(Segment::Title(pos..end));
        pos = end;
    }

    for kind in [CardKind::Cell, CardKind::Surface, CardKind::Data] {
        // The title line may itself have been blank, in which case that blank is
        // already accounted for and must not be emitted a second time.
        while blanks.get(blank_idx).is_some_and(|b| b.0 < pos) {
            blank_idx += 1;
        }
        let end = blanks.get(blank_idx).map_or(len, |b| b.0);
        if pos < end {
            segments.push(Segment::Block(pos..end, kind));
        }
        match blanks.get(blank_idx) {
            Some(&(bs, be)) => {
                segments.push(Segment::Blank(bs..be));
                pos = be;
                blank_idx += 1;
            }
            None => {
                pos = len;
                break;
            }
        }
    }

    // Anything after the data block's terminating blank line is preserved
    // verbatim: blank lines stay blank cards, content runs become Trailing.
    while pos < len {
        while blanks.get(blank_idx).is_some_and(|b| b.0 < pos) {
            blank_idx += 1;
        }
        let end = blanks.get(blank_idx).map_or(len, |b| b.0);
        if pos < end {
            segments.push(Segment::Block(pos..end, CardKind::Trailing));
        }
        match blanks.get(blank_idx) {
            Some(&(bs, be)) => {
                segments.push(Segment::Blank(bs..be));
                pos = be;
                blank_idx += 1;
            }
            None => break,
        }
    }

    segments
}

/// Segment one blank-line-free block, in parallel when it is large enough.
///
/// Chunk boundaries are snapped forward to a position where the sequential
/// segmenter would also have started a card (including any header comment run
/// that card absorbs), and never into the middle of a comment run — so each
/// chunk's absorb-backward/absorb-forward/flush decision is self-contained and
/// the concatenated result is identical to a single-threaded pass.
fn segment_block(src: &str, range: Range<usize>, kind: CardKind) -> Vec<Card> {
    let bytes = src.as_bytes();
    if range.len() < PARALLEL_THRESHOLD {
        let mut out = Vec::new();
        segment_run(src, range, kind, &mut out);
        return out;
    }

    let chunks = rayon::current_num_threads().max(1) * 4;
    let step = range.len() / chunks;
    let mut bounds = Vec::with_capacity(chunks + 1);
    bounds.push(range.start);
    for k in 1..chunks {
        let candidate = range.start + k * step;
        let snapped = find_card_start(bytes, range.start, range.end, candidate);
        if snapped > *bounds.last().unwrap() && snapped < range.end {
            bounds.push(snapped);
        }
    }
    bounds.push(range.end);

    let parts: Vec<Vec<Card>> = bounds
        .par_windows(2)
        .map(|w| {
            let mut out = Vec::new();
            segment_run(src, w[0]..w[1], kind, &mut out);
            out
        })
        .collect();

    let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for part in parts {
        out.extend(part);
    }
    out
}

/// The sequential segmenter for one contiguous run of lines.
fn segment_run(src: &str, range: Range<usize>, kind: CardKind, out: &mut Vec<Card>) {
    let bytes = src.as_bytes();
    // Start of a run of comment lines whose fate is not yet decided: they are
    // absorbed into the open card if a continuation follows, absorbed as the
    // header of the next card if a fresh (non-continuation) card line follows,
    // and become standalone Comment cards only when neither follows (a blank
    // line or the end of the range).
    let mut pending_comments: Option<usize> = None;
    // The card currently being extended by continuation lines.
    let mut open: Option<Range<usize>> = None;
    let mut prev_ends_amp = false;
    let mut pos = range.start;

    while pos < range.end {
        let end = line_end(bytes, pos).min(range.end);
        let line = &bytes[pos..end];

        if is_blank(line) {
            // Defensive: blocks are blank-line free, but a malformed file is
            // still segmented rather than rejected.
            close(src, &mut open, kind, out);
            flush_comments(src, &mut pending_comments, pos, out);
            out.push(Card::new(CardKind::Blank, src[pos..end].to_owned()));
            prev_ends_amp = false;
            pos = end;
            continue;
        }

        if is_comment_line(line) {
            pending_comments.get_or_insert(pos);
            pos = end;
            continue;
        }

        let continues = is_blank_col_continuation(line) || prev_ends_amp;
        match (continues, open.as_mut()) {
            (true, Some(card)) => {
                // Any pending comment lines sit between the card's previous line
                // and this one, so extending the range absorbs them for free —
                // the card's text is one contiguous slice either way.
                pending_comments = None;
                card.end = end;
            }
            _ => {
                close(src, &mut open, kind, out);
                // Any pending comment lines are this card's header: fold them
                // into its range instead of flushing them as standalone cards.
                let start = pending_comments.take().unwrap_or(pos);
                open = Some(start..end);
            }
        }

        prev_ends_amp = ends_with_ampersand(line);
        pos = end;
    }

    close(src, &mut open, kind, out);
    flush_comments(src, &mut pending_comments, range.end, out);
}

fn close(src: &str, open: &mut Option<Range<usize>>, kind: CardKind, out: &mut Vec<Card>) {
    if let Some(r) = open.take() {
        out.push(Card::new(kind, src[r].to_owned()));
    }
}

/// Emit undecided comment lines as standalone Comment cards, one per line.
fn flush_comments(src: &str, pending: &mut Option<usize>, upto: usize, out: &mut Vec<Card>) {
    let Some(start) = pending.take() else {
        return;
    };
    let bytes = src.as_bytes();
    let mut pos = start;
    while pos < upto {
        let end = line_end(bytes, pos).min(upto);
        out.push(Card::new(CardKind::Comment, src[pos..end].to_owned()));
        pos = end;
    }
}
