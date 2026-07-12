//! Whole-geometry renumbering.
//!
//! Renumbering must update **definitions and every reference** consistently:
//! renumbering surfaces rewrites each surface card's id *and* every signed
//! surface in cell geometry (preserving sense); renumbering cells rewrites each
//! cell's id *and* every `#n` complement and `LIKE n` base reference.
//!
//! Edits are applied through the CST integer-override overlay, so untouched
//! bytes stay exactly as written and re-emission is a single linear pass.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::cell::{cell_material, scan_cell_refs, RefKind};
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
pub fn renumber_surfaces(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
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
        for &(tok, val) in &edits {
            tree.set_token_int(tok, val);
        }
    }
}

/// Renumber every cell using `map` (old id → new id). Updates cell card
/// definitions, `#n` complement references, and `LIKE n` base references.
pub fn renumber_cells(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
    let mut edits: Vec<(u32, i64)> = Vec::new();
    for i in 0..ncards {
        edits.clear();
        scan_cell_refs(tree, i, |kind, tok, id| match kind {
            RefKind::CellId | RefKind::CellRef => edits.push((tok, map(id))),
            RefKind::SurfaceRef { .. } => {}
        });
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
pub fn renumber_materials(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
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
/// definitions and every surface's transform field (the periodic negative sign
/// is preserved).
pub fn renumber_transforms(tree: &mut GreenTree, mut map: impl FnMut(i64) -> i64) {
    let ncards = tree.cards().len();
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crunchy_syntax::parse;

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
