//! Whole-model renumbering (milestone M4).
//!
//! Each `renumber_*` remaps a family of ids through a caller-supplied
//! `Fn(i64) -> i64 + Sync` (an unmapped id maps to itself) and rewrites **both
//! the definitions and every reference** in one pass — the correctness property the
//! whole library hangs on (see `docs/03` "the distinction is essential for
//! renumbering"). A renumber never re-lexes the file: each touched card is
//! rewritten in place with [`Card::rewrite_tokens`], and only the token text
//! that actually changes is emitted, so an identity map is byte-for-byte lossless.
//!
//! The id index for the renumbered family is rebuilt afterward from the new ids;
//! references (cell material fields, surface senses, `#n` complements, ...) are
//! not indexed, so only the definition index needs rebuilding.

use compact_str::{format_compact, CompactString};
use migjorn_syntax::{Card, CardKind, SyntaxKind};
use rayon::prelude::*;

use crate::cell;
use crate::data;
use crate::model::{IdIndex, Model};
use crate::scan::{kind_at, sig, split_name};
use crate::surface;

/// Remap a numeric token that may carry a leading sign, preserving the sign (a
/// surface's sense, a periodic transform's `-`). Returns `None` if it is not a
/// plain signed integer.
///
/// Returns a `CompactString` rather than `String`: a signed `i64` is at most 20
/// bytes, comfortably within the inline capacity, so formatting one of the
/// millions of ids a whole-model renumber can touch never allocates.
pub(crate) fn remap_token<F: Fn(i64) -> i64>(text: &str, map: &F) -> Option<CompactString> {
    let (sign, digits) = if let Some(rest) = text.strip_prefix('-') {
        ("-", rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        ("+", rest)
    } else {
        ("", text)
    };
    let n: i64 = digits.parse().ok()?;
    Some(format_compact!("{sign}{}", map(n)))
}

/// Remap the id baked into a data-card name token (`m1` -> `m501`, `TR3` -> `TR9`,
/// `f4` -> `f14`), keeping the alphabetic part exactly as written.
pub(crate) fn remap_name<F: Fn(i64) -> i64>(name: &str, map: &F) -> Option<CompactString> {
    let (alpha, number) = split_name(name);
    let n = number?;
    Some(format_compact!("{alpha}{}", map(n)))
}

/// Push `(token, new_text)` only if the remap changed the token — so an identity
/// map produces no edits and the card is left untouched.
fn push_if_changed(
    edits: &mut Vec<(usize, CompactString)>,
    card: &Card,
    tok: usize,
    new: Option<CompactString>,
) {
    if let Some(new) = new {
        if new != card.token_text(tok) {
            edits.push((tok, new));
        }
    }
}

/// Which definition index a renumber rebuilds.
#[derive(Clone, Copy)]
enum Family {
    Cell,
    Surface,
    Material,
    Transform,
}

impl Model {
    // --- surfaces -----------------------------------------------------------

    /// Renumber surfaces: every surface definition (keeping a `+` white prefix)
    /// and every surface reference in cell geometry (keeping its sense, including
    /// the surfaces inside a `#( ... )` region complement).
    pub fn renumber_surfaces<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(Some(Family::Surface), |card| match card.kind() {
            CardKind::Surface => {
                let mut e = Vec::new();
                if let Some(tok) = surface::layout(card).id_tok {
                    push_if_changed(&mut e, card, tok, remap_token(card.token_text(tok), &map));
                }
                e
            }
            CardKind::Cell => {
                // A geometry number is a *surface* reference unless it directly
                // follows a `#` (which makes it a cell complement). Scanning the
                // tokens directly avoids `walk_geometry`'s per-term allocations —
                // the dominant cost of a whole-model renumber.
                let l = cell::layout(card);
                let toks = card.tokens();
                let start = l.geometry.start;
                let end = l.geometry.end.min(toks.len());
                let mut e = Vec::new();
                let mut after_hash = false;
                for (offset, t) in toks[start..end].iter().enumerate() {
                    if t.is_trivia() {
                        continue;
                    }
                    match t.kind {
                        SyntaxKind::Number => {
                            if !after_hash {
                                let i = start + offset;
                                let new = remap_token(card.token_text(i), &map);
                                push_if_changed(&mut e, card, i, new);
                            }
                            after_hash = false;
                        }
                        SyntaxKind::Hash => after_hash = true,
                        _ => after_hash = false,
                    }
                }
                e
            }
            _ => Vec::new(),
        });
    }

    /// Shift every surface id by `delta`, definitions and references together.
    pub fn offset_surfaces(&mut self, delta: i64) {
        self.renumber_surfaces(|i| i + delta);
    }

    // --- cells --------------------------------------------------------------

    /// Renumber cells: every cell definition, every `#n` cell complement, and
    /// every `LIKE n` base reference.
    pub fn renumber_cells<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(Some(Family::Cell), |card| {
            if card.kind() != CardKind::Cell {
                return Vec::new();
            }
            let l = cell::layout(card);
            let mut edits = Vec::new();

            // definition id
            if let Some(id_tok) = sig(card, 0) {
                push_if_changed(
                    &mut edits,
                    card,
                    id_tok,
                    remap_token(card.token_text(id_tok), &map),
                );
            }
            // `#n` complements: a `#` directly followed by a number (a bare `#(`
            // region carries no cell number). Direct token scan, no allocation.
            let toks = card.tokens();
            let start = l.geometry.start;
            let end = l.geometry.end.min(toks.len());
            let mut after_hash = false;
            for (offset, t) in toks[start..end].iter().enumerate() {
                if t.is_trivia() {
                    continue;
                }
                if t.kind == SyntaxKind::Hash {
                    after_hash = true;
                } else {
                    if after_hash && t.kind == SyntaxKind::Number {
                        let i = start + offset;
                        let new = remap_token(card.token_text(i), &map);
                        push_if_changed(&mut edits, card, i, new);
                    }
                    after_hash = false;
                }
            }
            // `LIKE n BUT` base
            if let Some(tok) = l.like_tok {
                push_if_changed(
                    &mut edits,
                    card,
                    tok,
                    remap_token(card.token_text(tok), &map),
                );
            }
            edits
        });
    }

    pub fn offset_cells(&mut self, delta: i64) {
        self.renumber_cells(|i| i + delta);
    }

    // --- materials ----------------------------------------------------------

    /// Renumber materials: `Mn` definitions, the `MTn`/`MXn` cards that reference
    /// a material, and every cell's material field (void `0` is left alone).
    pub fn renumber_materials<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(Some(Family::Material), |card| {
            let mut edits = Vec::new();
            match card.kind() {
                CardKind::Data => {
                    if let Some(h) = data::head(card) {
                        if matches!(h.mnemonic.as_str(), "m" | "mt" | "mx") {
                            let new = remap_name(card.token_text(h.name_tok), &map);
                            push_if_changed(&mut edits, card, h.name_tok, new);
                        }
                    }
                }
                CardKind::Cell => {
                    let l = cell::layout(card);
                    if let (Some(tok), Some(mat)) = (l.material_tok, l.material) {
                        if mat != 0 {
                            let new = remap_token(card.token_text(tok), &map);
                            push_if_changed(&mut edits, card, tok, new);
                        }
                    }
                }
                _ => {}
            }
            edits
        });
    }

    // --- transforms ---------------------------------------------------------

    /// Renumber transforms: `TRn`/`*TRn` definitions and every surface transform
    /// field (keeping a `-` periodic sign).
    pub fn renumber_transforms<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(Some(Family::Transform), |card| {
            let mut edits = Vec::new();
            match card.kind() {
                CardKind::Data => {
                    if let Some(h) = data::head(card) {
                        if h.mnemonic == "tr" {
                            let new = remap_name(card.token_text(h.name_tok), &map);
                            push_if_changed(&mut edits, card, h.name_tok, new);
                        }
                    }
                }
                CardKind::Surface => {
                    if let Some(tok) = surface::layout(card).transform_tok {
                        let new = remap_token(card.token_text(tok), &map);
                        push_if_changed(&mut edits, card, tok, new);
                    }
                }
                _ => {}
            }
            edits
        });
    }

    // --- universes & tallies (no definition index) --------------------------

    /// Renumber universes: every cell `u=` and every single-universe `fill=`
    /// (keeping any following `(transform)` group). Lattice `fill=` arrays
    /// (`fill=0:2 0:1 ...`) are left untouched — they name a grid of universes and
    /// need their own array-aware pass.
    pub fn renumber_universes<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(None, |card| {
            if card.kind() != CardKind::Cell {
                return Vec::new();
            }
            let l = cell::layout(card);
            let mut edits = Vec::new();
            for p in cell::params(card, &l.params) {
                let key = p.key.to_ascii_lowercase();
                let single = key == "u" || key == "fill";
                if !single {
                    continue;
                }
                // A `:` in the value means an index range: the lattice array form.
                let is_array = (p.value_tokens.start..p.value_tokens.end)
                    .any(|k| kind_at(card, k) == Some(SyntaxKind::Colon));
                if is_array {
                    continue;
                }
                if let Some(tok) = sig(card, p.value_tokens.start) {
                    if tok < p.value_tokens.end {
                        let new = remap_token(card.token_text(tok), &map);
                        push_if_changed(&mut edits, card, tok, new);
                    }
                }
            }
            edits
        });
    }

    /// Renumber tallies: the trailing id of every tally-family card (`Fn`, `FCn`,
    /// `FMn`, `En`, `Tn`, `Cn`, `SDn`, ...). Only cards whose number is actually
    /// remapped are touched, so the generous mnemonic set cannot disturb an
    /// unrelated card.
    ///
    /// This does **not** touch the cell/surface *bins* inside those cards (e.g.
    /// the `1 2 3` in `f4:n 1 2 3`) — [`Model::renumber_cells`] /
    /// [`Model::renumber_surfaces`] only scan `Cell`/`Surface` cards, not
    /// `Data` cards, so a bin referencing a renumbered cell or surface is left
    /// dangling. See `docs/03-mcnp-reference.md`.
    pub fn renumber_tallies<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        const TALLY: &[&str] = &[
            "f", "fc", "fm", "fs", "fq", "fu", "ft", "fic", "fip", "fir", "e", "t", "c", "sd",
            "de", "df", "em", "tm", "cm",
        ];
        self.renumber_pass(None, |card| {
            let mut edits = Vec::new();
            if card.kind() == CardKind::Data {
                if let Some(h) = data::head(card) {
                    if TALLY.contains(&h.mnemonic.as_str()) {
                        let new = remap_name(card.token_text(h.name_tok), &map);
                        push_if_changed(&mut edits, card, h.name_tok, new);
                    }
                }
            }
            edits
        });
    }

    // --- shared machinery ---------------------------------------------------

    /// Drive one renumber: `map` is `Fn + Sync` (a plain id -> id function, e.g. a
    /// constant offset or a pre-built table — never a Python callable, which
    /// would need the GIL and so could not be called concurrently), so both
    /// finding each card's token edits *and* applying them are embarrassingly
    /// parallel — no card's edits depend on another's. Finally rebuild the
    /// affected definition index, if any.
    fn renumber_pass(
        &mut self,
        family: Option<Family>,
        per_card: impl Fn(&Card) -> Vec<(usize, CompactString)> + Sync,
    ) {
        let cst = &self.cst;
        let all: Vec<(u32, Vec<(usize, CompactString)>)> = cst
            .order()
            .par_iter()
            .filter_map(|&slot| {
                let card = cst.card(slot)?;
                let mut edits = per_card(card);
                if edits.is_empty() {
                    return None;
                }
                edits.sort_by_key(|&(i, _)| i);
                Some((slot, edits))
            })
            .collect();
        self.cst.rewrite_many(all);
        if let Some(f) = family {
            self.reindex(f);
        }
    }

    /// Rebuild one definition index from the current cards after a renumber.
    ///
    ///
    /// The `(id, slot)` scan is parallel (and uses a cheap id read, not the full
    /// typed layout — a renumber only moved the id token), then a sequential
    /// first-wins insert preserves definition order. Without this, re-reading a
    /// million ids dominates the renumber it follows.
    fn reindex(&mut self, family: Family) {
        let cst = &self.cst;
        let pairs: Vec<(i64, u32)> = cst
            .order()
            .par_iter()
            .filter_map(|&slot| {
                let card = cst.card(slot)?;
                let id = match family {
                    Family::Cell => cheap_cell_id(card),
                    Family::Surface => cheap_surface_id(card),
                    Family::Material => data::head(card).and_then(|h| data::material_id(&h)),
                    Family::Transform => data::head(card).and_then(|h| data::transform_id(&h)),
                }?;
                Some((id, slot))
            })
            .collect();

        let mut fresh = IdIndex::default();
        fresh.reserve(pairs.len());
        for (id, slot) in pairs {
            fresh.entry(id).or_insert(slot);
        }
        let target = match family {
            Family::Cell => &mut self.cell_index,
            Family::Surface => &mut self.surface_index,
            Family::Material => &mut self.material_index,
            Family::Transform => &mut self.transform_index,
        };
        *target = fresh;
    }
}

/// Cheap cell id: the first significant token as a positive integer. Avoids the
/// full [`cell::layout`] (material/density/geometry/params) when only the id is
/// wanted, e.g. rebuilding the index after a renumber.
fn cheap_cell_id(card: &Card) -> Option<i64> {
    if card.kind() != CardKind::Cell {
        return None;
    }
    let i = sig(card, 0)?;
    crate::scan::int_at(card, i).filter(|&id| id > 0)
}

/// Cheap surface id: past an optional reflective `*`, the first token as a
/// positive integer (a `+` white prefix is absorbed by the integer parse).
fn cheap_surface_id(card: &Card) -> Option<i64> {
    if card.kind() != CardKind::Surface {
        return None;
    }
    let mut i = sig(card, 0)?;
    if kind_at(card, i) == Some(SyntaxKind::Star) {
        i = crate::scan::next(card, i)?;
    }
    crate::scan::int_at(card, i).filter(|&id| id > 0)
}

#[cfg(test)]
mod tests {
    use crate::Model;

    const SRC: &str = "t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n";

    #[test]
    fn renumber_surfaces_moves_defs_and_senses() {
        let mut m = Model::parse(SRC);
        m.renumber_surfaces(|i| i + 100);
        let out = m.to_source();
        assert!(out.contains("101 SO 5"), "{out}"); // definition
        assert!(out.contains("-101 imp:n=1"), "{out}"); // reference, sense preserved
                                                        // the index tracks the new id
        assert!(m.surface(101).is_some());
        assert!(m.surface(1).is_none());
    }

    #[test]
    fn renumber_cells_moves_defs_and_complements() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n2 0 1 #1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        // A dict-style mapping is a closure at the Rust layer (the binding adapts
        // a Python dict into one).
        m.renumber_cells(|i| if i == 1 { 501 } else { i });
        let out = m.to_source();
        assert!(out.contains("501 0 -1"), "{out}");
        assert!(out.contains("#501"), "{out}");
        assert!(m.cell(501).is_some());
    }

    #[test]
    fn renumber_is_lossless_under_identity() {
        let mut m = Model::parse(SRC);
        let src = m.to_source();
        m.renumber_surfaces(|i| i);
        assert_eq!(m.to_source(), src);
        m.renumber_cells(|i| i);
        assert_eq!(m.to_source(), src);
    }

    #[test]
    fn renumber_materials_moves_def_and_cell_field() {
        let mut m = Model::parse(SRC);
        m.renumber_materials(|i| if i == 1 { 7 } else { i });
        let out = m.to_source();
        assert!(out.contains("1 7 -1.0 -1"), "{out}"); // cell 1 material field 1 -> 7
        assert!(out.contains("m7 1001 1"), "{out}"); // Mn definition
        assert!(out.contains("2 0 1"), "{out}"); // void cell untouched
        assert!(m.material(7).is_some());
    }

    #[test]
    fn renumber_transforms_moves_def_and_surface_field() {
        let mut m =
            Model::parse("t\n1 0 -1 imp:n=1\n\n1 3 SO 5\n2 -3 PX 1\n\nm1 1001 1\ntr3 0 0 5\n");
        m.renumber_transforms(|i| i + 10);
        let out = m.to_source();
        assert!(out.contains("tr13 0 0 5"), "{out}"); // definition
        assert!(out.contains("1 13 SO 5"), "{out}"); // surface transform ref
        assert!(out.contains("2 -13 PX 1"), "{out}"); // periodic sign preserved
        assert!(m.transform(13).is_some());
    }

    #[test]
    fn renumber_universes_moves_u_and_fill() {
        let mut m = Model::parse(
            "t\n1 0 -1 u=2 imp:n=1\n2 0 -2 fill=2 imp:n=1\n\n1 SO 5\n2 SO 9\n\nm1 1001 1\n",
        );
        m.renumber_universes(|i| i + 40);
        let out = m.to_source();
        assert!(out.contains("u=42"), "{out}");
        assert!(out.contains("fill=42"), "{out}");
    }

    #[test]
    fn renumber_tallies_moves_the_family() {
        let mut m = Model::parse(
            "t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\nf4:n 1\nfc4 a comment\ne4 1 10\n",
        );
        m.renumber_tallies(|i| if i == 4 { 14 } else { i });
        let out = m.to_source();
        assert!(out.contains("f14:n 1"), "{out}");
        assert!(out.contains("fc14 a comment"), "{out}");
        assert!(out.contains("e14 1 10"), "{out}");
    }

    #[test]
    fn offset_helpers_shift_everything() {
        let mut m = Model::parse(SRC);
        m.offset_surfaces(1000);
        assert!(m.to_source().contains("1001 SO 5"));
        assert!(m.to_source().contains("-1001 imp:n=1"));
        m.offset_cells(50);
        assert!(m.cell(51).is_some());
        assert!(m.cell(52).is_some());
    }
}
