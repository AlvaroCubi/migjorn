//! Whole-geometry renumbering.
//!
//! Renumbering must update **definitions and every reference** consistently:
//! renumbering surfaces rewrites each surface card's id *and* every signed
//! surface in cell geometry (preserving sense); renumbering cells rewrites each
//! cell's id *and* every `#n` complement and `LIKE n` base reference.
//!
//! Edits are applied through the CST integer-override overlay, so untouched
//! bytes stay exactly as written and re-emission is a single linear pass.

use crunchy_syntax::GreenTree;

use crate::cell::{scan_cell_refs, RefKind};
use crate::surface::surface_id;

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
}
