//! Whole-geometry renumbering.
//!
//! Renumbering must update **definitions and every reference** consistently:
//! renumbering surfaces rewrites each surface card's id *and* every signed
//! surface in cell geometry (preserving sense); renumbering cells rewrites each
//! cell's id *and* every `#n` complement and `LIKE n` base reference.
//!
//! Edits are applied through the CST integer-override overlay, so untouched
//! bytes stay exactly as written and re-emission is a single linear pass.

use migjorn_syntax::{GreenTree, SyntaxKind};

use crate::cell::{cell_material, scan_cell_params, scan_cell_refs, RefKind};
use crate::material::parse_material;
use crate::num::parse_int;
use crate::surface::{parse_surface, surface_id};
use crate::transform::parse_transform;

/// Renumber every surface using `map` (old id → new id). Updates surface card
/// definitions and every surface reference in cell geometry; sense is preserved
/// and `+`/white-boundary prefixes are kept.
///
/// `map` is applied to the *original* ids; the pass is single-shot, so a map
/// need not be injective, but a non-injective map can of course alias surfaces.
pub(crate) fn renumber_surfaces(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    // Reused across cards to avoid per-card allocation.
    let mut edits: Vec<(u32, i64)> = Vec::new();
    for i in 0..ncards {
        // Surface definitions (light header read; no coefficient parsing).
        if let Some((id_token, id, white)) = surface_id(tree, i) {
            let new = map(id);
            if white {
                tree.set_token_text(id_token, format!("+{new}"));
            } else {
                tree.set_token_int(id_token, new);
            }
        }
        // Surface references inside cell geometry (allocation-light scan).
        edits.clear();
        scan_cell_refs(tree, i, |kind, tok, val| {
            if let RefKind::SurfaceRef { negative } = kind {
                let new = map(val);
                edits.push((tok, if negative { -new } else { new }));
            }
        });
        // Surface ids in surface-tally (F1/F2) bins (sign = direction, kept).
        push_tally_bin_edits(tree, i, TallyKind::Surface, &mut map, &mut edits);
        for &(tok, val) in &edits {
            tree.set_token_int(tok, val);
        }
    }
}

/// Renumber every cell using `map` (old id → new id). Updates cell card
/// definitions, `#n` complement references, and `LIKE n` base references.
pub(crate) fn renumber_cells(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    let mut edits: Vec<(u32, i64)> = Vec::new();
    for i in 0..ncards {
        edits.clear();
        scan_cell_refs(tree, i, |kind, tok, id| match kind {
            RefKind::CellId | RefKind::CellRef => edits.push((tok, map(id))),
            RefKind::SurfaceRef { .. } => {}
        });
        // Cell ids in cell-tally (F4/F6/F7/F8) bins.
        push_tally_bin_edits(tree, i, TallyKind::Cell, &mut map, &mut edits);
        for &(tok, val) in &edits {
            tree.set_token_int(tok, val);
        }
    }
}

/// Rewrite the trailing digits of a mnemonic token to `new`, preserving the
/// non-digit prefix and its case (`m1`→`m124`, `MT2`→`MT124`, `tr3`→`tr103`).
fn rewrite_trailing_number(tree: &mut GreenTree, tok: u32, new: i64) {
    let text = tree.token_text(tok).into_owned();
    let ndigits = text
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .count();
    let prefix = &text[..text.len() - ndigits];
    tree.set_token_text(tok, format!("{prefix}{new}"));
}

/// If card `i` is an `MTn`/`MXn` data card (a reference to material `n`), return
/// its mnemonic token and the referenced material number.
fn mt_mx_material_ref(tree: &GreenTree, i: usize) -> Option<(u32, i64)> {
    let card = tree.cards()[i];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let tok = tree.card_content_tokens(&card).next()?;
    if tree.token_kind(tok) != SyntaxKind::IDENT {
        return None;
    }
    let text = tree.token_text(tok);
    let up = text.to_ascii_uppercase();
    let digits = up.strip_prefix("MT").or_else(|| up.strip_prefix("MX"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((tok, parse_int(digits)?))
}

/// Renumber every material using `map` (old id → new id). Updates `Mn`
/// definitions, the material field of every cell, and `MTn`/`MXn` cards that
/// reference a material. Void cells (material 0) are left unchanged.
pub(crate) fn renumber_materials(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    for i in 0..ncards {
        // `Mn` definition.
        if let Some(m) = parse_material(tree, i) {
            let new = map(m.id);
            rewrite_trailing_number(tree, m.id_token, new);
            continue;
        }
        // `MTn` / `MXn` reference.
        if let Some((tok, id)) = mt_mx_material_ref(tree, i) {
            let new = map(id);
            rewrite_trailing_number(tree, tok, new);
            continue;
        }
        // Cell material field (skip void).
        if let Some((tok, mat)) = cell_material(tree, i) {
            if mat != 0 {
                tree.set_token_int(tok, map(mat));
            }
        }
    }
}

/// Renumber every transform using `map` (old id → new id). Updates `TRn`/`*TRn`
/// definitions, every surface's transform field, a cell's `trcl=n`/`*trcl=n`
/// reference, and each parenthesised single-number transform in a cell `fill=`
/// value. Signs (periodic surface transform, negative `trcl`) are preserved;
/// inline `trcl`/`fill` transform lists are left untouched.
pub(crate) fn renumber_transforms(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    let mut edits: Vec<(u32, i64)> = Vec::new();
    for i in 0..ncards {
        // `TRn` / `*TRn` definition.
        if let Some(t) = parse_transform(tree, i) {
            let new = map(t.id);
            rewrite_trailing_number(tree, t.id_token, new);
            continue;
        }
        // Surface transform field (magnitude mapped; sign kept).
        if let Some(s) = parse_surface(tree, i) {
            if let (Some(tok), Some(val)) = (s.transform_token, s.transform) {
                let new = map(val.abs());
                tree.set_token_int(tok, if val < 0 { -new } else { new });
            }
            continue;
        }
        // Cell parameters that reference a transform: a bare `trcl=n`/`*trcl=n`
        // and each parenthesised single-number transform group in a `fill=`
        // value (`fill= u (n)` or a lattice entry `(n)`).
        edits.clear();
        scan_cell_params(tree, i, |p| {
            let key = tree.token_text(p.keyword);
            if key.eq_ignore_ascii_case("trcl") {
                if let Some(tok) = trcl_transform_token(tree, p.value_tokens) {
                    push_signed_ref_edit(tree, tok, &mut map, &mut edits);
                }
            } else if key.eq_ignore_ascii_case("fill") {
                for tok in fill_transform_tokens(tree, p.value_tokens) {
                    push_signed_ref_edit(tree, tok, &mut map, &mut edits);
                }
            }
        });
        for &(tok, val) in &edits {
            tree.set_token_int(tok, val);
        }
    }
}

/// The transform-number token referenced by a `trcl=` value, or `None` when the
/// value is an inline transform (a parenthesised or multi-number list) rather
/// than a bare `TRn` reference.
fn trcl_transform_token(tree: &GreenTree, values: &[u32]) -> Option<u32> {
    let mut num = None;
    for &t in values {
        match tree.token_kind(t) {
            SyntaxKind::NUMBER => {
                if num.is_some() {
                    return None; // more than one number => inline transform list
                }
                num = Some(t);
            }
            SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => return None, // inline
            _ => {}
        }
    }
    num
}

/// The transform-number tokens inside a `fill=` value: each parenthesised group
/// holding exactly one number (`fill= u (n)`, or a lattice entry `(n)`) is a
/// `TRn` reference. Multi-number parens are inline transforms and are skipped.
fn fill_transform_tokens(tree: &GreenTree, values: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < values.len() {
        if tree.token_kind(values[i]) != SyntaxKind::L_PAREN {
            i += 1;
            continue;
        }
        // Collect the numbers up to the matching `)`.
        let mut j = i + 1;
        let mut only = None;
        let mut count = 0u32;
        while j < values.len() && tree.token_kind(values[j]) != SyntaxKind::R_PAREN {
            if tree.token_kind(values[j]) == SyntaxKind::NUMBER {
                count += 1;
                only = Some(values[j]);
            }
            j += 1;
        }
        if count == 1 {
            out.push(only.unwrap());
        }
        i = j + 1;
    }
    out
}

/// The universe-number tokens inside a `fill=` value list. Handles a simple
/// `fill=N` (optionally `N (TR…)`) and a lattice array
/// `fill= i1:i2 j1:j2 k1:k2  u u u …` (the universe entries after the ranges;
/// parenthesised transform groups are skipped).
fn fill_universe_tokens(tree: &GreenTree, values: &[u32]) -> Vec<u32> {
    let has_range = values
        .iter()
        .any(|&t| tree.token_kind(t) == SyntaxKind::COLON);
    if !has_range {
        // Simple fill: the first number before any parenthesised group.
        for &t in values {
            match tree.token_kind(t) {
                SyntaxKind::NUMBER => return vec![t],
                SyntaxKind::L_PAREN => break,
                _ => {}
            }
        }
        return Vec::new();
    }
    // Lattice fill: consume leading `lo : hi` range triples, then take the
    // universe numbers that follow (outside any parenthesised group).
    let mut k = 0;
    while k + 2 < values.len()
        && tree.token_kind(values[k]) == SyntaxKind::NUMBER
        && tree.token_kind(values[k + 1]) == SyntaxKind::COLON
        && tree.token_kind(values[k + 2]) == SyntaxKind::NUMBER
    {
        k += 3;
    }
    let mut out = Vec::new();
    let mut depth = 0u32;
    for &t in &values[k..] {
        match tree.token_kind(t) {
            SyntaxKind::L_PAREN => depth += 1,
            SyntaxKind::R_PAREN => depth = depth.saturating_sub(1),
            SyntaxKind::NUMBER if depth == 0 => out.push(t),
            _ => {}
        }
    }
    out
}

/// Renumber every universe using `map` (old id → new id). Updates `u=`
/// definitions and `fill=` references (single fills and lattice fill arrays);
/// the sign of a `u=` value is preserved. Universe 0 (the real world) is left
/// unchanged.
pub(crate) fn renumber_universes(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    let mut edits: Vec<(u32, i64)> = Vec::new();
    for i in 0..ncards {
        edits.clear();
        scan_cell_params(tree, i, |p| {
            let key = tree.token_text(p.keyword);
            if key.eq_ignore_ascii_case("u") {
                // `u=` takes the first number (with sign preserved).
                if let Some(&tok) = p
                    .value_tokens
                    .iter()
                    .find(|&&t| tree.token_kind(t) == SyntaxKind::NUMBER)
                {
                    push_signed_ref_edit(tree, tok, &mut map, &mut edits);
                }
            } else if key.eq_ignore_ascii_case("fill") {
                for tok in fill_universe_tokens(tree, p.value_tokens) {
                    push_signed_ref_edit(tree, tok, &mut map, &mut edits);
                }
            }
        });
        for &(tok, val) in &edits {
            tree.set_token_int(tok, val);
        }
    }
}

/// Queue a renumber of the signed reference number at `tok` (magnitude mapped,
/// sign preserved, 0 skipped). Shared by universe and transform reference edits.
fn push_signed_ref_edit(
    tree: &GreenTree,
    tok: u32,
    map: &mut impl FnMut(i64) -> i64,
    edits: &mut Vec<(u32, i64)>,
) {
    if let Some(v) = parse_int(&tree.token_text(tok)) {
        if v != 0 {
            let new = map(v.abs());
            edits.push((tok, if v < 0 { -new } else { new }));
        }
    }
}

/// What a tally's bins reference, from its type (the last digit of the number).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TallyKind {
    /// F1/F2 — surface bins.
    Surface,
    /// F4/F6/F7/F8 — cell bins.
    Cell,
    /// F5 — point detector (coordinates, not cell/surface ids).
    Detector,
    /// Any other trailing digit.
    Other,
}

/// A parsed `Fn` tally card: its number token, number, kind, and the id tokens
/// of its cell/surface bin list.
struct Tally {
    number_token: u32,
    number: i64,
    kind: TallyKind,
    bin_tokens: Vec<u32>,
}

/// Parse the `Fn` tally card at `i` (`*Fn`, `Fn:p` handled), or `None` if the
/// card is not an `Fn` tally. Cheap for non-data cards (kind check first), so it
/// is safe to call on every card in the hot renumber loops.
fn parse_tally(tree: &GreenTree, i: usize) -> Option<Tally> {
    let card = tree.cards()[i];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let toks: Vec<u32> = tree
        .card_content_tokens(&card)
        .filter(|&t| tree.token_kind(t) != SyntaxKind::AMP)
        .collect();
    let mut p = 0;
    if toks.first().map(|&t| tree.token_kind(t)) == Some(SyntaxKind::STAR) {
        p = 1;
    }
    let name_tok = *toks.get(p)?;
    if tree.token_kind(name_tok) != SyntaxKind::IDENT {
        return None;
    }
    let up = tree.token_text(name_tok).to_ascii_uppercase();
    // Exactly `F` + digits (not FC/FM/FS/...); those are companion cards.
    let digits = up.strip_prefix('F')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = parse_int(digits)?;
    p += 1;
    // Optional `:particle`.
    if toks.get(p).map(|&t| tree.token_kind(t)) == Some(SyntaxKind::COLON) {
        p += 1;
        if toks.get(p).map(|&t| tree.token_kind(t)) == Some(SyntaxKind::IDENT) {
            p += 1;
        }
    }
    let kind = match number % 10 {
        1 | 2 => TallyKind::Surface,
        4 | 6 | 7 | 8 => TallyKind::Cell,
        5 => TallyKind::Detector,
        _ => TallyKind::Other,
    };
    let bin_tokens = toks[p..]
        .iter()
        .copied()
        .filter(|&t| tree.token_kind(t) == SyntaxKind::NUMBER)
        .collect();
    Some(Tally {
        number_token: name_tok,
        number,
        kind,
        bin_tokens,
    })
}

/// Tally companion-card prefixes that carry the tally number (`E5`, `FM5`, …).
/// Matched with an all-digit remainder, so a longer prefix wins (`EM5` matches
/// `EM`, not `E`).
const TALLY_PREFIXES: &[&str] = &[
    "FC", "FM", "FS", "FQ", "FT", "FU", "FIC", "FIP", "FIR", "FMESH", "SD", "DE", "DF", "EM", "TM",
    "CM", "TF", "DD", "E", "T", "C",
];

/// If card `i` is a tally companion card (`<prefix><digits>`), return its
/// mnemonic token and the referenced tally number.
fn tally_companion_ref(tree: &GreenTree, i: usize) -> Option<(u32, i64)> {
    let card = tree.cards()[i];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let mut it = tree.card_content_tokens(&card);
    let mut tok = it.next()?;
    if tree.token_kind(tok) == SyntaxKind::STAR {
        tok = it.next()?;
    }
    if tree.token_kind(tok) != SyntaxKind::IDENT {
        return None;
    }
    let up = tree.token_text(tok).to_ascii_uppercase();
    for pre in TALLY_PREFIXES {
        if let Some(rest) = up.strip_prefix(pre) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return Some((tok, parse_int(rest)?));
            }
        }
    }
    None
}

/// If card `i` is a tally of `want` kind, queue renumber edits for every id in
/// its bin list (magnitude mapped; sign preserved; 0 skipped). Called from the
/// cell/surface renumber passes so tally bins stay consistent.
fn push_tally_bin_edits(
    tree: &GreenTree,
    i: usize,
    want: TallyKind,
    map: &mut impl FnMut(i64) -> i64,
    edits: &mut Vec<(u32, i64)>,
) {
    let Some(t) = parse_tally(tree, i) else {
        return;
    };
    if t.kind != want {
        return;
    }
    for tok in t.bin_tokens {
        push_signed_ref_edit(tree, tok, map, edits);
    }
}

/// Renumber every tally using `map` (old id → new id). Updates each `Fn` tally
/// number and common companion cards (`FC`, `FM`, `FS`, `E`, `T`, `C`, `SD`, …).
/// Note: this renumbers the tally *ids*; the cell/surface ids inside tally bins
/// are updated by [`renumber_cells`]/[`renumber_surfaces`] instead.
pub(crate) fn renumber_tallies(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    for i in 0..ncards {
        if let Some(t) = parse_tally(tree, i) {
            rewrite_trailing_number(tree, t.number_token, map(t.number));
            continue;
        }
        if let Some((tok, id)) = tally_companion_ref(tree, i) {
            rewrite_trailing_number(tree, tok, map(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migjorn_syntax::parse;

    #[test]
    fn renumber_surfaces_updates_defs_and_refs() {
        let src = "title\n1 0 -1 2 #(3) imp:n=1\n\n1 SO 5\n2 PX 0\n3 PY 0\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_surfaces(&mut tree, |id| id + 1000);
        let out = tree.to_source();
        // Definitions shifted.
        assert!(out.contains("1001 SO 5"));
        assert!(out.contains("1002 PX 0"));
        assert!(out.contains("1003 PY 0"));
        // References shifted, sense preserved.
        assert!(out.contains("1 0 -1001 1002 #(1003) imp:n=1"));
    }

    #[test]
    fn renumber_cells_updates_defs_and_complements() {
        let src = "title\n1 0 -1 imp:n=1\n2 0 1 #1 imp:n=1\n7 LIKE 1 BUT imp:n=2\n\n1 SO 5\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_cells(&mut tree, |id| id + 500);
        let out = tree.to_source();
        assert!(out.contains("501 0 -1 imp:n=1"));
        assert!(out.contains("502 0 1 #501 imp:n=1")); // #1 -> #501
        assert!(out.contains("507 LIKE 501 BUT imp:n=2")); // base cell 1 -> 501
    }

    #[test]
    fn renumber_is_otherwise_lossless() {
        // Everything except the renumbered tokens is byte-identical, including
        // comments and spacing.
        let src =
            "title\nc keep me\n1 0 -1   2  imp:n=1 $ trailing\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_surfaces(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("c keep me"));
        assert!(out.contains("$ trailing"));
        assert!(out.contains("1 0 -11   12  imp:n=1")); // spacing preserved
        assert!(out.contains("11 SO 5"));
    }

    #[test]
    fn renumber_materials_updates_defs_refs_and_mt() {
        let src =
            "title\n1 5 -1.0 -1 imp:n=1\n2 0 1 imp:n=1\n\n1 SO 5\n\nm5 1001 1\nmt5 lwtr.10t\n";
        let mut tree = parse(src).tree;
        renumber_materials(&mut tree, |id| id + 100);
        let out = tree.to_source();
        assert!(out.contains("m105 1001 1")); // Mn definition
        assert!(out.contains("mt105 lwtr.10t")); // MT reference
        assert!(out.contains("1 105 -1.0 -1 imp:n=1")); // cell material field
        assert!(out.contains("2 0 1 imp:n=1")); // void left unchanged
    }

    #[test]
    fn renumber_transforms_updates_defs_and_surface_field() {
        let src = "title\n1 0 -1\n\n1 3 PZ 50\n2 -3 SO 5\n\nm1 1001 1\ntr3 0 0 5\n*tr7 0 0 0\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr13 0 0 5")); // TR definition
        assert!(out.contains("*tr17 0 0 0")); // *TR definition, star preserved
        assert!(out.contains("1 13 PZ 50")); // surface transform field
        assert!(out.contains("2 -13 SO 5")); // periodic sign preserved
    }

    #[test]
    fn renumber_transforms_updates_cell_trcl() {
        // A bare `trcl=n` (and `*trcl=n`) references TR n; a negative sign is kept.
        let src = "title\n1 0 -1 trcl=3 imp:n=1\n2 0 -2 *trcl=-3 imp:n=1\n\n1 SO 5\n2 SO 9\n\nm1 1001 1\ntr3 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr13 0 0 5"), "def: {out}"); // definition
        assert!(out.contains("1 0 -1 trcl=13 imp:n=1"), "trcl: {out}");
        assert!(
            out.contains("2 0 -2 *trcl=-13 imp:n=1"),
            "*trcl sign: {out}"
        );
    }

    #[test]
    fn renumber_transforms_skips_inline_trcl() {
        // An inline `trcl=(...)` is a transform *list*, not a reference: untouched.
        let src = "title\n1 0 -1 trcl=(1 0 0) imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr11 0 0 5")); // definition still renumbered
        assert!(
            out.contains("1 0 -1 trcl=(1 0 0) imp:n=1"),
            "inline kept: {out}"
        );
    }

    #[test]
    fn renumber_transforms_updates_fill_transform() {
        // A parenthesised single number after a fill universe is a TR reference;
        // the universe itself must not be touched by the transform pass.
        let src = "title\n1 0 -1 fill=5 (3) imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr3 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr13 0 0 5")); // definition
        assert!(
            out.contains("1 0 -1 fill=5 (13) imp:n=1"),
            "fill trcl: {out}"
        );
    }

    #[test]
    fn renumber_transforms_skips_inline_fill_transform() {
        // A multi-number parenthesised group in fill is an inline transform: kept.
        let src = "title\n1 0 -1 fill=5 (0 0 5) imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr11 0 0 5")); // definition
        assert!(
            out.contains("1 0 -1 fill=5 (0 0 5) imp:n=1"),
            "inline kept: {out}"
        );
    }

    #[test]
    fn renumber_transforms_lattice_fill_per_entry() {
        // Lattice fill with a per-entry transform `(3)`: universe entries stay,
        // only the transform reference is renumbered.
        let src = "title\n1 0 -1 lat=1 fill=0:1 0:0 0:0 5 6 (3) imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr3 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("tr13 0 0 5")); // definition
                                             // Universes 5, 6 untouched; transform 3 -> 13.
        assert!(
            out.contains("fill=0:1 0:0 0:0 5 6 (13) imp:n=1"),
            "lattice: {out}"
        );
    }

    #[test]
    fn renumber_transforms_is_otherwise_lossless() {
        let src = "title\n1 0 -1 trcl=3 fill=5 (3) imp:n=1\n\n1 3 SO 5\n\nm1 1001 1\ntr3 0 0 5\n";
        let mut tree = parse(src).tree;
        renumber_transforms(&mut tree, |id| id);
        assert_eq!(tree.to_source(), src);
    }

    #[test]
    fn renumber_universes_updates_u_and_fill() {
        let src =
            "title\n1 0 -1 u=5 imp:n=1\n2 0 -2 fill=5 imp:n=1\n\n1 SO 5\n2 SO 9\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_universes(&mut tree, |id| id + 100);
        let out = tree.to_source();
        assert!(out.contains("1 0 -1 u=105 imp:n=1"), "got: {out}");
        assert!(out.contains("2 0 -2 fill=105 imp:n=1"), "got: {out}");
    }

    #[test]
    fn renumber_universes_lattice_fill_array() {
        // A lattice cell whose fill array references universes 5 and 6.
        let src = "title\n1 0 -1 lat=1 fill=0:1 0:0 0:0 5 6 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_universes(&mut tree, |id| id + 10);
        let out = tree.to_source();
        // Ranges untouched; universe entries renumbered.
        assert!(out.contains("fill=0:1 0:0 0:0 15 16"), "got: {out}");
    }

    #[test]
    fn renumber_universes_preserves_sign_and_zero() {
        let src = "title\n1 0 -1 u=-3 fill=0 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_universes(&mut tree, |id| id + 20);
        let out = tree.to_source();
        assert!(out.contains("u=-23"), "sign not preserved: {out}"); // -3 -> -23
        assert!(out.contains("fill=0"), "universe 0 must not change: {out}");
    }

    #[test]
    fn renumber_tallies_updates_number_and_companions() {
        let src = "title\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\nf4:n 1\nfc4 flux\ne4 1 10\nsd4 1\n";
        let mut tree = parse(src).tree;
        renumber_tallies(&mut tree, |id| id + 10);
        let out = tree.to_source();
        assert!(out.contains("f14:n 1")); // tally number
        assert!(out.contains("fc14 flux")); // FC comment card
        assert!(out.contains("e14 1 10")); // energy bins
        assert!(out.contains("sd14 1")); // segment divisor
    }

    #[test]
    fn renumber_cells_updates_cell_tally_bins() {
        let src = "title\n1 0 -1\n2 0 1\n\n1 SO 5\n\nm1 1001 1\nf4:n 1 2\n";
        let mut tree = parse(src).tree;
        renumber_cells(&mut tree, |id| id + 100);
        let out = tree.to_source();
        assert!(out.contains("101 0 -1")); // cell def
        assert!(out.contains("f4:n 101 102")); // cell-flux tally bins follow
    }

    #[test]
    fn renumber_surfaces_updates_surface_tally_bins() {
        let src = "title\n1 0 -1\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\nf1:n 1 -2\n";
        let mut tree = parse(src).tree;
        renumber_surfaces(&mut tree, |id| id + 100);
        let out = tree.to_source();
        assert!(out.contains("101 SO 5")); // surface def
        assert!(out.contains("f1:n 101 -102")); // surface-current tally bins (sign kept)
    }

    #[test]
    fn renumber_cells_leaves_surface_tally_bins_alone() {
        // A surface tally's bins are surface ids -- renumbering cells must not
        // touch them.
        let src = "title\n1 0 -1\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\nf2:n 1 2\n";
        let mut tree = parse(src).tree;
        renumber_cells(&mut tree, |id| id + 100);
        assert!(tree.to_source().contains("f2:n 1 2"));
    }

    #[test]
    fn renumber_materials_is_lossless_elsewhere() {
        let src = "title\n1 5 -1.0 -1 imp:n=1 $ keep\n\n1 SO 5\n\nm5 1001 1\n";
        let mut tree = parse(src).tree;
        renumber_materials(&mut tree, |id| id + 1);
        let out = tree.to_source();
        assert!(out.contains("$ keep"));
        assert!(out.contains("1 6 -1.0 -1 imp:n=1"));
        assert!(out.contains("m6 1001 1"));
    }
}
