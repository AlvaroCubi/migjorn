//! The concrete syntax tree: an ordered list of self-contained cards.

use compact_str::CompactString;
use rayon::prelude::*;

use crate::card::Card;
use crate::kind::{CardKind, Eol};
use crate::segment::segment;

/// Below this many bytes emission runs on one thread.
const PARALLEL_EMIT_THRESHOLD: usize = 1 << 22;

/// The whole model's syntax.
///
/// Cards live in an append-only `arena` indexed by slot, with `order` giving
/// file order as a list of slots. That split is what makes all three of the
/// design's requirements hold at once:
///
/// * **slot -> card is O(1)** and never invalidated — it is a direct index.
/// * **structural edits are cheap** — inserting or removing a card moves four
///   bytes per card in `order`, not a 56-byte `Card` struct per card, and there
///   is no side index to repair.
/// * **emission stays sequential** — `arena` starts in file order, so only the
///   cards a user has added are out of place.
///
/// There is no shared source buffer and no global token arena; the input text is
/// not retained after parsing.
#[derive(Clone)]
pub struct Cst {
    arena: Vec<Option<Card>>,
    order: Vec<u32>,
    /// Kind of each card in `order`, kept in lockstep. A contiguous byte per card
    /// so `end_of_block` (which every `add_*` calls) scans this instead of chasing
    /// scattered `arena` entries — the difference between a cache-friendly walk and
    /// hundreds of thousands of cache misses on a large model.
    kinds: Vec<CardKind>,
    eol: Eol,
}

impl Cst {
    /// Lex `src` into self-contained cards.
    ///
    /// The input is only borrowed for the duration of this call. Cards are
    /// independent, so blocks are segmented in parallel; the result is identical
    /// to a single-threaded pass because chunk boundaries are snapped to
    /// positions where a card genuinely starts.
    pub fn parse(src: &str) -> Cst {
        let cards = segment(src);
        Cst::from_cards(cards, Eol::detect(src))
    }

    /// Build a `Cst` directly from already-lexed cards, in the given order — no
    /// lexing happens here.
    ///
    /// For a caller that already holds parsed [`Card`]s (e.g. cloning a subset
    /// of another model's cards) this skips the relex that `parse` would pay
    /// for the same content. See `docs/05-parallelism-overhead.md` — this is
    /// what makes it possible to drop a card block without a text round trip.
    pub fn from_cards(mut cards: Vec<Card>, eol: Eol) -> Cst {
        for (i, card) in cards.iter_mut().enumerate() {
            card.slot = i as u32;
        }
        // Reserve growth headroom so the first structural edit does not pay a full
        // reallocation of the (large) arena/order/kinds vectors. `collect()` would
        // size them exactly to the card count, making the very first `add_*` copy
        // the whole arena; the slack amortizes that away.
        let n = cards.len();
        let slack = n / 8 + 16;
        let mut order = Vec::with_capacity(n + slack);
        order.extend(0..n as u32);
        let mut kinds = Vec::with_capacity(n + slack);
        kinds.extend(cards.iter().map(Card::kind));
        let mut arena = Vec::with_capacity(n + slack);
        arena.extend(cards.into_iter().map(Some));
        Cst {
            arena,
            order,
            kinds,
            eol,
        }
    }

    /// The line terminator this file uses, for cards added later.
    #[inline]
    pub fn eol(&self) -> Eol {
        self.eol
    }

    /// Number of live cards.
    #[inline]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Slots in file order.
    #[inline]
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// Resolve a stable slot. `None` once that card has been removed.
    #[inline]
    pub fn card(&self, slot: u32) -> Option<&Card> {
        self.arena.get(slot as usize)?.as_ref()
    }

    #[inline]
    pub fn card_mut(&mut self, slot: u32) -> Option<&mut Card> {
        self.arena.get_mut(slot as usize)?.as_mut()
    }

    /// Cards in file order.
    pub fn cards(&self) -> impl Iterator<Item = &Card> + '_ {
        self.order.iter().filter_map(move |&s| self.card(s))
    }

    /// Consume the `Cst`, yielding its cards in file order by value.
    ///
    /// Unlike [`cards`](Self::cards), this moves each `Card` out — tokens it
    /// already has included — instead of borrowing it. A caller about to fold
    /// these cards into another `Cst` uses this to skip re-lexing content
    /// that was already lexed once.
    pub fn into_cards(self) -> impl Iterator<Item = Card> {
        let Cst {
            mut arena, order, ..
        } = self;
        order
            .into_iter()
            .filter_map(move |s| arena[s as usize].take())
    }

    /// The card at file position `i`.
    #[inline]
    pub fn at(&self, i: usize) -> Option<&Card> {
        self.card(*self.order.get(i)?)
    }

    /// Current file position of a slot.
    ///
    /// A linear scan of `order` — 4 bytes per card, sequential and cache-
    /// friendly (~0.3 ms over a million cards), which beats maintaining a
    /// slot -> position side table that every insert would have to scatter-write.
    pub fn position_of(&self, slot: u32) -> Option<usize> {
        self.order.iter().position(|&s| s == slot)
    }

    /// Total emitted size in bytes.
    pub fn len_bytes(&self) -> usize {
        self.cards().map(Card::len_bytes).sum()
    }

    /// Re-emit the whole model.
    ///
    /// Every card emits the same way — a copy of its own bytes — whether or not
    /// it has been edited. There is no fast path to fall off, so losslessness is
    /// unconditional and emission cost is independent of how many edits were
    /// made.
    ///
    /// The parallel path writes each chunk of cards straight into its own
    /// disjoint slice of the final buffer. Rendering chunks into separate
    /// `String`s and concatenating them afterwards would copy the whole model
    /// twice, which on a 380 MB file costs more than the parallelism saves.
    pub fn to_source(&self) -> String {
        let chunk_size = (self.order.len() / (rayon::current_num_threads().max(1) * 4)).max(1);
        let chunks: Vec<&[u32]> = self.order.chunks(chunk_size).collect();

        let sizes: Vec<usize> = chunks
            .par_iter()
            .map(|slots| {
                slots
                    .iter()
                    .filter_map(|&s| self.card(s))
                    .map(Card::len_bytes)
                    .sum()
            })
            .collect();
        let total: usize = sizes.iter().sum();

        if total < PARALLEL_EMIT_THRESHOLD || chunks.len() < 2 {
            let mut out = String::with_capacity(total);
            for card in self.cards() {
                out.push_str(card.text());
            }
            return out;
        }

        let mut buf = vec![0u8; total];
        let mut pieces: Vec<&mut [u8]> = Vec::with_capacity(chunks.len());
        let mut rest = buf.as_mut_slice();
        for &size in &sizes {
            let (head, tail) = rest.split_at_mut(size);
            pieces.push(head);
            rest = tail;
        }

        pieces.into_par_iter().zip(chunks).for_each(|(dst, slots)| {
            let mut at = 0usize;
            for &slot in slots {
                if let Some(card) = self.card(slot) {
                    let bytes = card.text().as_bytes();
                    dst[at..at + bytes.len()].copy_from_slice(bytes);
                    at += bytes.len();
                }
            }
            debug_assert_eq!(at, dst.len());
        });

        // SAFETY: every byte written came from a `&str` (each card's `text`), and
        // cards are concatenated at character boundaries, so the result is valid
        // UTF-8 by construction. Re-validating would cost an extra pass over the
        // whole model for no information.
        unsafe { String::from_utf8_unchecked(buf) }
    }

    // --- structural editing: `order` operations, never a relex ---------------

    /// Build a detached card. O(snippet length) — microseconds.
    pub fn new_card(kind: CardKind, text: String) -> Card {
        Card::new(kind, text)
    }

    /// Insert a card at file position `i`, returning its fresh slot.
    ///
    /// Costs one `Vec<u32>` memmove; nothing is re-lexed and no other card is
    /// touched.
    pub fn insert_at(&mut self, i: usize, mut card: Card) -> u32 {
        let slot = self.arena.len() as u32;
        card.slot = slot;
        let kind = card.kind();
        let at = i.min(self.order.len());
        self.arena.push(Some(card));
        self.order.insert(at, slot);
        self.kinds.insert(at, kind);
        slot
    }

    /// Append a card at the end of the model.
    pub fn push(&mut self, card: Card) -> u32 {
        self.insert_at(self.order.len(), card)
    }

    /// Insert several cards at file position `i` in one go, returning their
    /// fresh slots in the same order.
    ///
    /// One `Vec::splice` per call instead of one `Vec::insert` per card: an
    /// `insert_at` loop shifts everything after `i` once per card, which is
    /// O(n) per call and O(n·m) for m cards on a large model (this is what a
    /// whole-model merge used to pay, one card at a time). Splicing the whole
    /// batch in shifts the tail exactly once, for O(n + m) total.
    pub fn insert_many_at(&mut self, i: usize, cards: impl IntoIterator<Item = Card>) -> Vec<u32> {
        let at = i.min(self.order.len());
        let mut slots = Vec::new();
        let mut kinds = Vec::new();
        for mut card in cards {
            let slot = self.arena.len() as u32;
            card.slot = slot;
            kinds.push(card.kind());
            slots.push(slot);
            self.arena.push(Some(card));
        }
        self.order.splice(at..at, slots.iter().copied());
        self.kinds.splice(at..at, kinds);
        slots
    }

    /// Remove the card at file position `i`, returning its slot. The slot is
    /// tombstoned and never reused, so handles to it fail cleanly.
    pub fn remove_at(&mut self, i: usize) -> Option<u32> {
        if i >= self.order.len() {
            return None;
        }
        let slot = self.order.remove(i);
        self.kinds.remove(i);
        self.arena[slot as usize] = None;
        Some(slot)
    }

    /// Remove a card by slot.
    pub fn remove_slot(&mut self, slot: u32) -> bool {
        match self.position_of(slot) {
            Some(i) => self.remove_at(i).is_some(),
            None => false,
        }
    }

    /// Apply per-card token rewrites in parallel.
    ///
    /// Each entry is `(slot, edits)` where `edits` is sorted by token index; the
    /// rewrite of one card touches only that card's own text and tokens, so
    /// distinct slots are disjoint `&mut` into the arena and rayon can drive them
    /// at once. This is what keeps a whole-model renumber — up to millions of
    /// token rewrites, each rebuilding a card's `String` — inside the budget.
    /// Each replacement token is a `CompactString` rather than `String`: an id
    /// never exceeds its inline capacity, so building millions of them (one per
    /// changed reference) never touches the heap.
    pub fn rewrite_many(&mut self, edits: Vec<(u32, Vec<(usize, CompactString)>)>) {
        if edits.is_empty() {
            return;
        }
        // Scatter into a slot-indexed table so the parallel pass over the arena
        // can find each card's edits by position with no shared lookup.
        let mut by_slot: Vec<Option<Vec<(usize, CompactString)>>> = std::iter::repeat_with(|| None)
            .take(self.arena.len())
            .collect();
        for (slot, e) in edits {
            by_slot[slot as usize] = Some(e);
        }
        self.arena
            .par_iter_mut()
            .zip(by_slot.into_par_iter())
            .for_each(|(card, edit)| {
                if let (Some(card), Some(edit)) = (card.as_mut(), edit) {
                    let refs: Vec<(usize, &str)> =
                        edit.iter().map(|(i, s)| (*i, s.as_str())).collect();
                    card.rewrite_tokens(&refs);
                }
            });
    }

    /// Position just past the last card of `kind` — where a new card of that
    /// kind belongs.
    pub fn end_of_block(&self, kind: CardKind) -> Option<usize> {
        self.kinds.iter().rposition(|&k| k == kind).map(|i| i + 1)
    }
}

impl std::fmt::Debug for Cst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cst")
            .field("cards", &self.order.len())
            .field("bytes", &self.len_bytes())
            .field("eol", &self.eol)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "Minimal valid model\n\
                       1 1 -1.0 -1 imp:n=1 $ fuel sphere\n\
                       2 0 1 imp:n=0\n\
                       \n\
                       1 SO 5\n\
                       \n\
                       m1 1001.31c 1\n";

    #[test]
    fn round_trips_and_classifies_blocks() {
        let cst = Cst::parse(SRC);
        assert_eq!(cst.to_source(), SRC);
        let kinds: Vec<CardKind> = cst.cards().map(Card::kind).collect();
        assert_eq!(
            kinds,
            vec![
                CardKind::Title,
                CardKind::Cell,
                CardKind::Cell,
                CardKind::Blank,
                CardKind::Surface,
                CardKind::Blank,
                CardKind::Data,
            ]
        );
    }

    #[test]
    fn slots_survive_edits_to_other_cards() {
        let mut cst = Cst::parse(SRC);
        let second_cell = cst.at(2).unwrap().slot();
        // insert ahead of it; the handle must still resolve to the same card
        let new = Cst::new_card(CardKind::Cell, "3 0 -1 imp:n=1\n".to_owned());
        cst.insert_at(1, new);
        assert_eq!(cst.card(second_cell).unwrap().text(), "2 0 1 imp:n=0\n");
        assert_eq!(cst.position_of(second_cell), Some(3));
    }

    #[test]
    fn insert_then_remove_is_byte_identity() {
        let mut cst = Cst::parse(SRC);
        let at = cst.end_of_block(CardKind::Cell).unwrap();
        let slot = cst.insert_at(
            at,
            Cst::new_card(CardKind::Cell, "42 0 -1 imp:n=1\n".to_owned()),
        );
        assert!(cst.to_source().contains("42 0 -1 imp:n=1"));
        assert!(cst.remove_slot(slot));
        assert_eq!(cst.to_source(), SRC);
        // the slot is tombstoned, not reused
        assert!(cst.card(slot).is_none());
    }

    #[test]
    fn insert_many_at_batches_in_order() {
        let mut cst = Cst::parse(SRC);
        let at = cst.end_of_block(CardKind::Cell).unwrap();
        let cards = (10..13).map(|n| Cst::new_card(CardKind::Cell, format!("{n} 0 -1 imp:n=1\n")));
        let slots = cst.insert_many_at(at, cards);

        // one fresh, distinct slot per card, in the order given
        assert_eq!(slots.len(), 3);
        let mut sorted = slots.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);

        // landed contiguously at `at`, in the same order as the input
        for (offset, slot) in slots.iter().enumerate() {
            assert_eq!(cst.position_of(*slot), Some(at + offset));
            assert_eq!(cst.card(*slot).unwrap().kind(), CardKind::Cell);
        }
        assert_eq!(cst.card(slots[0]).unwrap().text(), "10 0 -1 imp:n=1\n");
        assert_eq!(cst.card(slots[2]).unwrap().text(), "12 0 -1 imp:n=1\n");

        // a single batched insert is byte-for-byte equivalent to three inserts
        let mut expected = Cst::parse(SRC);
        for n in 10..13 {
            let at = expected.end_of_block(CardKind::Cell).unwrap();
            expected.insert_at(
                at,
                Cst::new_card(CardKind::Cell, format!("{n} 0 -1 imp:n=1\n")),
            );
        }
        assert_eq!(cst.to_source(), expected.to_source());
    }

    #[test]
    fn insert_many_at_with_empty_iter_is_a_no_op() {
        let mut cst = Cst::parse(SRC);
        let at = cst.end_of_block(CardKind::Cell).unwrap();
        let slots = cst.insert_many_at(at, std::iter::empty());
        assert!(slots.is_empty());
        assert_eq!(cst.to_source(), SRC);
    }

    #[test]
    fn removed_slots_are_never_reused() {
        let mut cst = Cst::parse(SRC);
        let a = cst.push(Cst::new_card(CardKind::Data, "nps 1e6\n".to_owned()));
        cst.remove_slot(a);
        let b = cst.push(Cst::new_card(CardKind::Data, "print\n".to_owned()));
        assert_ne!(a, b);
        assert!(cst.card(a).is_none());
    }
}
