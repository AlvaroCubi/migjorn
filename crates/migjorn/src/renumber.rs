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

/// Remap a surface reference in cell geometry. Like [`remap_token`], but also
/// accepts a macrobody facet reference (`-470.1`: facet 1 of macrobody 470),
/// remapping the macrobody number and keeping the `.k` suffix.
pub(crate) fn remap_surface_ref<F: Fn(i64) -> i64>(text: &str, map: &F) -> Option<CompactString> {
    match text.split_once('.') {
        Some((whole, facet)) if !facet.is_empty() && facet.bytes().all(|b| b.is_ascii_digit()) => {
            let new = remap_token(whole, map)?;
            Some(format_compact!("{new}.{facet}"))
        }
        _ => remap_token(text, map),
    }
}

/// [`remap_surface_ref`] for a tally bin, which may be glued to the start of a
/// lattice index (`2[0`): only the part before the `[` is an id.
fn remap_bin<F: Fn(i64) -> i64>(text: &str, map: &F) -> Option<CompactString> {
    match text.split_once('[') {
        Some((id, rest)) if !id.is_empty() => {
            Some(format_compact!("{}[{rest}", remap_surface_ref(id, map)?))
        }
        Some(_) => None,
        None => remap_surface_ref(text, map),
    }
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

/// `nI` data shortcut (`n` evenly spaced values between its neighbours): the
/// count `n`, or `None` if the token is anything else (`3R`, `4ILOG`, `7`).
fn interp_count(text: &str) -> Option<i64> {
    let digits = text.strip_suffix(['i', 'I'])?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Edits for the `nI` interpolation shortcuts among the tokens `range` of a
/// card's id list (fill-array universes, tally bins).
///
/// `a 2I b` stands for `a, a+d, a+2d, b`; the intermediate ids are implicit, so
/// a map that is not linear over them would silently change them. When the
/// mapped endpoints still interpolate to the mapped intermediates the shortcut
/// is kept; otherwise it is replaced by the explicit mapped values.
/// `skip_parens` ignores parenthesised groups (a fill array's `(tr)` groups).
fn interpolation_edits<F: Fn(i64) -> i64>(
    card: &Card,
    range: std::ops::Range<usize>,
    skip_parens: bool,
    map: &F,
) -> Vec<(usize, CompactString)> {
    let mut edits = Vec::new();
    let (mut depth, mut brackets) = (0i32, false);
    let mut last: Option<i64> = None;
    let mut pending: Option<(usize, i64, i64)> = None; // (token, n, a)
    let mut i = range.start;
    while let Some(k) = sig(card, i) {
        if k >= range.end {
            break;
        }
        i = k + 1;
        let text = card.token_text(k);
        match card.tokens()[k].kind {
            SyntaxKind::LParen => depth += 1,
            SyntaxKind::RParen => depth -= 1,
            _ if text.contains('[') => brackets = true,
            _ if text.contains(']') => brackets = false,
            SyntaxKind::Number if !brackets && !(skip_parens && depth > 0) => {
                let near_colon = [crate::scan::prev(card, k), crate::scan::next(card, k)]
                    .into_iter()
                    .flatten()
                    .any(|j| kind_at(card, j) == Some(SyntaxKind::Colon));
                if near_colon {
                    continue;
                }
                if let Some(n) = interp_count(text) {
                    if let Some(a) = last {
                        pending = Some((k, n, a));
                    }
                } else if let Some(v) = crate::scan::parse_int(text) {
                    if let Some((tok, n, a)) = pending.take() {
                        if let Some(e) = interpolate(tok, n, a, v, map) {
                            edits.push(e);
                        }
                    }
                    last = Some(v);
                } else {
                    // another shortcut (`3R`, `2J`): the neighbours are unknown
                    pending = None;
                    last = None;
                }
            }
            _ => {}
        }
    }
    edits
}

fn interpolate<F: Fn(i64) -> i64>(
    tok: usize,
    n: i64,
    a: i64,
    b: i64,
    map: &F,
) -> Option<(usize, CompactString)> {
    let steps = n + 1;
    if n <= 0 || (b - a) % steps != 0 {
        return None; // not an integer progression; leave it as written
    }
    let d = (b - a) / steps;
    let mapped: Vec<i64> = (1..=n).map(|j| map(a + j * d)).collect();
    let (ma, mb) = (map(a), map(b));
    let linear = (mb - ma) % steps == 0
        && mapped
            .iter()
            .enumerate()
            .all(|(j, &m)| m == ma + (j as i64 + 1) * ((mb - ma) / steps));
    if linear {
        return None;
    }
    let text: Vec<String> = mapped.iter().map(i64::to_string).collect();
    Some((tok, CompactString::from(text.join(" "))))
}

/// The bins of an `Fn` tally card: whether they are surfaces (`F1`/`F2`) or
/// cells (`F4`/`F6`/`F7`/`F8`), and the token range holding them. Other tally
/// types (`F5` detectors, ...) have no cell/surface bins.
fn tally_bins(card: &Card) -> Option<(bool, std::ops::Range<usize>)> {
    if card.kind() != CardKind::Data {
        return None;
    }
    let h = data::head(card)?;
    if h.mnemonic != "f" {
        return None;
    }
    let surfaces = match h.number? % 10 {
        1 | 2 => true,
        4 | 6 | 7 | 8 => false,
        _ => return None,
    };
    Some((surfaces, h.values_start..card.tokens().len()))
}

/// Bin ids of a tally card, as token indices: plain numbers outside `[...]`
/// lattice indices, and not the `n` of a `nR`-style shortcut.
fn tally_bin_tokens(card: &Card, range: std::ops::Range<usize>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut brackets = false;
    let mut i = range.start;
    while let Some(k) = sig(card, i) {
        if k >= range.end {
            break;
        }
        i = k + 1;
        let text = card.token_text(k);
        // `2[0` is one word: the id, then the start of a lattice index.
        if !brackets && card.tokens()[k].kind == SyntaxKind::Number {
            out.push(k);
        }
        if text.contains('[') {
            brackets = true;
        }
        if text.contains(']') {
            brackets = false;
        }
    }
    out
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
                                let new = remap_surface_ref(card.token_text(i), &map);
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
            CardKind::Data => {
                let mut e = Vec::new();
                if let Some((true, range)) = tally_bins(card) {
                    for tok in tally_bin_tokens(card, range.clone()) {
                        let new = remap_bin(card.token_text(tok), &map);
                        push_if_changed(&mut e, card, tok, new);
                    }
                    e.extend(interpolation_edits(card, range, false, &map));
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
            if let Some((false, range)) = tally_bins(card) {
                let mut edits = Vec::new();
                for tok in tally_bin_tokens(card, range.clone()) {
                    let new = remap_bin(card.token_text(tok), &map);
                    push_if_changed(&mut edits, card, tok, new);
                }
                edits.extend(interpolation_edits(card, range, false, &map));
                return edits;
            }
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
                CardKind::Cell => {
                    // `fill=u (n)`, `trcl=n` / `trcl=(n)` and the `(n)` groups
                    // inside a lattice fill array.
                    let l = cell::layout(card);
                    for p in cell::params(card, &l.params) {
                        for tok in cell::param_refs(card, &p).transforms {
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

    // --- universes & tallies (no definition index) --------------------------

    /// Renumber universes: every cell `u=`, every single-universe `fill=`
    /// (keeping any following `(transform)` group) and every universe entry of a
    /// lattice `fill=` array (`fill=0:2 0:1 0:0 5 6 7`). Array shortcuts
    /// (`3R`, `2J`, ...) and `(transform)` groups are left alone.
    pub fn renumber_universes<F: Fn(i64) -> i64 + Sync>(&mut self, map: F) {
        self.renumber_pass(None, |card| {
            if card.kind() != CardKind::Cell {
                return Vec::new();
            }
            let l = cell::layout(card);
            let mut edits = Vec::new();
            for p in cell::params(card, &l.params) {
                if p.key.eq_ignore_ascii_case("u") {
                    if let Some(tok) = sig(card, p.value_tokens.start) {
                        if tok < p.value_tokens.end {
                            let new = remap_token(card.token_text(tok), &map);
                            push_if_changed(&mut edits, card, tok, new);
                        }
                    }
                } else {
                    for tok in cell::param_refs(card, &p).universes {
                        let new = remap_token(card.token_text(tok), &map);
                        push_if_changed(&mut edits, card, tok, new);
                    }
                    if p.key.eq_ignore_ascii_case("fill") {
                        edits.extend(interpolation_edits(
                            card,
                            p.value_tokens.clone(),
                            true,
                            &map,
                        ));
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
    /// This does **not** touch the cell/surface *bins* inside those cards; that is
    /// the job of [`Model::renumber_cells`] / [`Model::renumber_surfaces`], which
    /// rewrite the bins of `F1`/`F2` (surfaces, facets included) and
    /// `F4`/`F6`/`F7`/`F8` (cells) cards.
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
    fn renumber_facets_transforms_and_arrays() {
        let src = "t\n1 0 -470.1 +470.2 imp:n=1\n2 0 -2 fill=3 (7) trcl=8 imp:n=1\n\
3 0 -2 lat=1 u=4 fill=0:1 0:0 0:0 3 (7) 2R 5(9) imp:n=1\n4 0 -2 *trcl=(0 0 5) fill=3 (1 2 3) imp:n=1\n\n\
470 RPP -1 1 -1 1 -1 1\n2 SO 9\n\nm1 1001 1\ntr7 0 0 1\n";
        let mut m = Model::parse(src);
        m.renumber_surfaces(|i| i + 1000);
        assert!(
            m.to_source().contains("-1470.1 +1470.2"),
            "{}",
            m.to_source()
        );
        m.renumber_transforms(|i| i + 100);
        let out = m.to_source();
        assert!(out.contains("fill=3 (107) trcl=108"), "{out}");
        assert!(out.contains("3 (107) 2R 5(109)"), "{out}");
        assert!(out.contains("*trcl=(0 0 5) fill=3 (1 2 3)"), "{out}");
        m.renumber_universes(|i| i + 50);
        let out = m.to_source();
        assert!(
            out.contains("u=54 fill=0:1 0:0 0:0 53 (107) 2R 55(109)"),
            "{out}"
        );
        assert!(out.contains("fill=53 (107)"), "{out}");
    }

    #[test]
    fn interpolation_shortcuts_are_expanded_when_not_preserved() {
        let src = "t\n3 0 -2 lat=1 u=4 fill=0:3 0:0 0:0 1 2I 4 imp:n=1\n\n2 SO 9\n\nm1 1001 1\n";
        let mut m = Model::parse(src);
        m.renumber_universes(|i| i + 10); // linear: shortcut kept
        assert!(m.to_source().contains("11 2I 14"), "{}", m.to_source());
        let mut m = Model::parse(src);
        m.renumber_universes(|i| if i == 2 { 77 } else { i + 10 });
        assert!(
            m.to_source().contains("fill=0:3 0:0 0:0 11 77 13 14"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn renumber_tally_bins() {
        let src = "t\n1 0 -1 imp:n=1\n2 0 -470.1 imp:n=1\n\n1 SO 5\n470 RPP -1 1 -1 1 -1 1\n\n\
m1 1001 1\nf1:n 470.1 470.2 1\nf2:n,p (1 470.3) T\nf4:n 1<2[0 0 0] (1 2) 1 2I 7\nf5:n 1 1 1 0.1\n";
        let mut m = Model::parse(src);
        m.renumber_surfaces(|i| i + 1000);
        m.renumber_cells(|i| i + 500);
        let out = m.to_source();
        assert!(out.contains("f1:n 1470.1 1470.2 1001"), "{out}");
        assert!(out.contains("f2:n,p (1001 1470.3) T"), "{out}");
        assert!(
            out.contains("f4:n 501<502[0 0 0] (501 502) 501 2I 507"),
            "{out}"
        );
        assert!(out.contains("f5:n 1 1 1 0.1"), "{out}");
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
