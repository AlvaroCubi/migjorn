//! Emit typed nodes back to MCNP source text.
//!
//! This is the counterpart to parsing: given an edited/owned node, produce the
//! card body that re-parses to the same structure. Most edits are now expressed
//! as lossless token splices on the CST (see `model.rs`); this whole-card emit
//! is the **fallback** used only where a splice is not available — a brand-new
//! card from `add_cell`, or removing a surface from a nested union. Its job is
//! correctness of the edited card, not preserving original formatting (which the
//! fallback does not attempt to keep).

use crate::cell::{GeomExpr, OwnedCell};

/// Precedence of a geometry node, for deciding when a child needs parentheses.
/// Union binds loosest, then intersection (juxtaposition), then a factor
/// (surface, `#n`, or a `#(...)`/parenthesised region).
fn precedence(e: &GeomExpr) -> u8 {
    match e {
        GeomExpr::Union(_) => 1,
        GeomExpr::Intersection(_) => 2,
        _ => 3,
    }
}

/// Emit `child`, wrapping it in parentheses if its precedence is looser than
/// `min` (i.e. it would otherwise re-associate incorrectly in the parent).
fn emit_child(child: &GeomExpr, min: u8) -> String {
    let s = emit_geometry(child);
    if precedence(child) < min {
        format!("({s})")
    } else {
        s
    }
}

/// Emit a geometry region as MCNP text (senses, `#` complements, unions with
/// `:`, intersections by juxtaposition, parenthesised where precedence needs).
pub(crate) fn emit_geometry(e: &GeomExpr) -> String {
    match e {
        GeomExpr::Surface(s) => {
            if s.negative {
                format!("-{}", s.id)
            } else {
                s.id.to_string()
            }
        }
        GeomExpr::CellComplement(c) => format!("#{}", c.id),
        GeomExpr::Complement(inner) => format!("#({})", emit_geometry(inner)),
        GeomExpr::Intersection(parts) => parts
            .iter()
            .map(|p| emit_child(p, 2))
            .collect::<Vec<_>>()
            .join(" "),
        GeomExpr::Union(parts) => parts
            .iter()
            .map(|p| emit_child(p, 1))
            .collect::<Vec<_>>()
            .join(" : "),
        // An Error node has no faithful text; callers must not emit malformed
        // geometry (promotion refuses it), so this is a defensive fallback.
        GeomExpr::Error => String::new(),
    }
}

/// Emit an owned cell card body: `id [material [density]] geometry [params]`.
/// A void cell (material 0) has no density; the parameter tail is appended
/// verbatim.
pub(crate) fn emit_cell(c: &OwnedCell) -> String {
    let mut s = format!("{} ", c.id);
    match (c.material, c.density) {
        // Void, or a `LIKE`-shaped node that lost its material (shouldn't occur).
        (Some(0), _) | (None, _) => s.push_str("0 "),
        (Some(m), Some(d)) => s.push_str(&format!("{m} {d} ")),
        // Non-void but density missing: emit the material and let the geometry
        // follow (best-effort; promotion only accepts well-formed cells).
        (Some(m), None) => s.push_str(&format!("{m} ")),
    }
    s.push_str(&emit_geometry(&c.geometry));
    if !c.params_text.is_empty() {
        s.push(' ');
        s.push_str(&c.params_text);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{cells, promote_cell};
    use crunchy_syntax::parse;

    fn owned(cell_line: &str) -> OwnedCell {
        let src = format!("title\n{cell_line}\n\n1 PX 0\n\nm1 1001 1\n");
        let tree = parse(src).tree;
        let ci = cells(&tree).next().unwrap().card_index;
        promote_cell(&tree, ci).expect("promotable cell")
    }

    /// Emitting a cell, re-parsing, and re-emitting must be idempotent — the
    /// geometry survives the round-trip. (We compare emitted text rather than
    /// the trees, since `GeomExpr` equality includes source token indices, which
    /// legitimately differ between two parses.)
    fn roundtrip_geometry(cell_line: &str) {
        let oc = owned(cell_line);
        let emitted = emit_cell(&oc);
        let src = format!("title\n{emitted}\n\n1 PX 0\n\nm1 1001 1\n");
        let tree = parse(src).tree;
        let re = promote_cell(&tree, cells(&tree).next().unwrap().card_index)
            .expect("re-parse promotable");
        assert!(!re.geometry.is_empty());
        assert_eq!(
            emit_geometry(&re.geometry),
            emit_geometry(&oc.geometry),
            "geometry changed through emit: {emitted}"
        );
    }

    #[test]
    fn emits_simple_intersection() {
        assert_eq!(emit_geometry(&owned("1 0 -1 2 -3").geometry), "-1 2 -3");
    }

    #[test]
    fn emits_union_and_parenthesises_in_intersection() {
        // (1 : 2) 3  — the union must stay parenthesised inside the intersection.
        let g = owned("1 0 (1 : 2) 3").geometry;
        assert_eq!(emit_geometry(&g), "(1 : 2) 3");
    }

    #[test]
    fn emits_complements() {
        assert_eq!(
            emit_geometry(&owned("5 0 #3 #(1 -2)").geometry),
            "#3 #(1 -2)"
        );
    }

    #[test]
    fn emit_cell_keeps_material_density_and_params() {
        // The inline `$` comment is trivia (preserved by the CST card overlay,
        // not by the owned node), so it is not part of `emit_cell`; the typed
        // parameter tail (`imp:n=1`) is.
        let oc = owned("1 1 -1.0 -1 2 imp:n=1 $ fuel");
        let out = emit_cell(&oc);
        assert!(out.starts_with("1 1 -1 -1 2 "), "got: {out}");
        assert!(out.contains("imp:n=1"), "params tail dropped: {out}");
    }

    #[test]
    fn emit_cell_void_has_no_density() {
        let oc = owned("2 0 1 -2 imp:n=0");
        let out = emit_cell(&oc);
        assert!(out.starts_with("2 0 1 -2"), "got: {out}");
    }

    #[test]
    fn geometry_roundtrips() {
        roundtrip_geometry("1 0 -1 2 -3");
        roundtrip_geometry("1 0 1 -2 : 3");
        roundtrip_geometry("1 0 (1 : (2 3)) -4");
        roundtrip_geometry("5 0 #3 #(1 -2)");
    }
}
