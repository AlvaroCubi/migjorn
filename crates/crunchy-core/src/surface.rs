//! Typed view of surface cards.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::num::{parse_float, parse_int};

/// A surface mnemonic. `Other` preserves anything we don't recognise (including
/// macrobody variants not yet enumerated) so parsing never fails on it.
///
/// `#[non_exhaustive]`: new named mnemonics may be added in future releases, so
/// downstream matches should include a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceKind {
    // Planes
    P,
    Px,
    Py,
    Pz,
    // Spheres
    So,
    S,
    Sx,
    Sy,
    Sz,
    // Cylinders
    Cx,
    Cy,
    Cz,
    CParX,
    CParY,
    CParZ,
    // Cones
    Kx,
    Ky,
    Kz,
    KParX,
    KParY,
    KParZ,
    // Quadrics / tori
    Sq,
    Gq,
    Tx,
    Ty,
    Tz,
    // Point-defined axisymmetric surfaces
    X,
    Y,
    Z,
    // Macrobodies
    Box,
    Rpp,
    Sph,
    Rcc,
    Rhp,
    Hex,
    Rec,
    Trc,
    Ell,
    Wed,
    Arb,
    /// An unrecognised mnemonic, preserved verbatim (uppercased).
    Other(Box<str>),
}

impl SurfaceKind {
    /// The canonical MCNP mnemonic string for this kind.
    pub fn mnemonic(&self) -> String {
        use SurfaceKind::*;
        let s = match self {
            P => "P",
            Px => "PX",
            Py => "PY",
            Pz => "PZ",
            So => "SO",
            S => "S",
            Sx => "SX",
            Sy => "SY",
            Sz => "SZ",
            Cx => "CX",
            Cy => "CY",
            Cz => "CZ",
            CParX => "C/X",
            CParY => "C/Y",
            CParZ => "C/Z",
            Kx => "KX",
            Ky => "KY",
            Kz => "KZ",
            KParX => "K/X",
            KParY => "K/Y",
            KParZ => "K/Z",
            Sq => "SQ",
            Gq => "GQ",
            Tx => "TX",
            Ty => "TY",
            Tz => "TZ",
            X => "X",
            Y => "Y",
            Z => "Z",
            Box => "BOX",
            Rpp => "RPP",
            Sph => "SPH",
            Rcc => "RCC",
            Rhp => "RHP",
            Hex => "HEX",
            Rec => "REC",
            Trc => "TRC",
            Ell => "ELL",
            Wed => "WED",
            Arb => "ARB",
            Other(s) => return s.to_string(),
        };
        s.to_string()
    }

    /// Parse a mnemonic case-insensitively.
    pub fn from_mnemonic(s: &str) -> SurfaceKind {
        use SurfaceKind::*;
        // Uppercase into a small stack buffer-free String (mnemonics are tiny).
        let up = s.to_ascii_uppercase();
        match up.as_str() {
            "P" => P,
            "PX" => Px,
            "PY" => Py,
            "PZ" => Pz,
            "SO" => So,
            "S" => S,
            "SX" => Sx,
            "SY" => Sy,
            "SZ" => Sz,
            "CX" => Cx,
            "CY" => Cy,
            "CZ" => Cz,
            "C/X" => CParX,
            "C/Y" => CParY,
            "C/Z" => CParZ,
            "KX" => Kx,
            "KY" => Ky,
            "KZ" => Kz,
            "K/X" => KParX,
            "K/Y" => KParY,
            "K/Z" => KParZ,
            "SQ" => Sq,
            "GQ" => Gq,
            "TX" => Tx,
            "TY" => Ty,
            "TZ" => Tz,
            "X" => X,
            "Y" => Y,
            "Z" => Z,
            "BOX" => Box,
            "RPP" => Rpp,
            "SPH" => Sph,
            "RCC" => Rcc,
            "RHP" => Rhp,
            "HEX" => Hex,
            "REC" => Rec,
            "TRC" => Trc,
            "ELL" => Ell,
            "WED" => Wed,
            "ARB" => Arb,
            _ => Other(up.into_boxed_str()),
        }
    }
}

/// A parsed surface card.
#[derive(Debug, Clone)]
pub struct Surface {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Surface number.
    pub id: i64,
    /// Token index of the surface id (anchor for adding a transform number).
    pub(crate) id_token: u32,
    /// Optional transformation number; negative denotes a periodic surface.
    pub transform: Option<i64>,
    /// Token index of the transform number, if present (for renumbering).
    pub(crate) transform_token: Option<u32>,
    /// Leading `*` — a reflective boundary.
    pub reflective: bool,
    /// Leading `+` on the id — a white boundary.
    pub white: bool,
    /// The surface mnemonic.
    pub kind: SurfaceKind,
    /// Coefficients that parsed successfully.
    pub coeffs: Vec<f64>,
    /// Token index of each coefficient in `coeffs` (parallel to it), for
    /// in-place value edits.
    pub(crate) coeff_tokens: Vec<u32>,
    /// False if a coefficient token failed to parse (e.g. an unexpanded
    /// repeat shortcut like `2R`); such cards need shortcut expansion first.
    pub well_formed: bool,
}

/// Parse the surface card at `card_index`, or `None` if it does not look like a
/// surface (no id + mnemonic).
pub(crate) fn parse_surface(tree: &GreenTree, card_index: usize) -> Option<Surface> {
    let card = &tree.cards()[card_index];
    if card.kind != SyntaxKind::SURFACE_CARD {
        return None;
    }
    let toks: Vec<u32> = tree.card_content_tokens(card).collect();
    let mut pos = 0usize;

    let mut reflective = false;
    if toks.get(pos).map(|&i| tree.token_kind(i)) == Some(SyntaxKind::STAR) {
        reflective = true;
        pos += 1;
    }

    // Surface id.
    let id_tok = *toks.get(pos)?;
    if tree.token_kind(id_tok) != SyntaxKind::NUMBER {
        return None;
    }
    let id_text = tree.token_text(id_tok);
    let white = id_text.starts_with('+');
    let id = parse_int(&id_text)?;
    pos += 1;

    // Optional transform number (a NUMBER before the mnemonic).
    let mut transform = None;
    let mut transform_token = None;
    if let Some(&t) = toks.get(pos) {
        if tree.token_kind(t) == SyntaxKind::NUMBER {
            transform = parse_int(&tree.token_text(t));
            transform_token = Some(t);
            pos += 1;
        }
    }

    // Mnemonic.
    let mnem_tok = *toks.get(pos)?;
    if tree.token_kind(mnem_tok) != SyntaxKind::IDENT {
        return None;
    }
    let kind = SurfaceKind::from_mnemonic(&tree.token_text(mnem_tok));
    pos += 1;

    // Coefficients. `&` is a line-continuation marker, not a coefficient.
    let mut coeffs = Vec::with_capacity(toks.len() - pos);
    let mut coeff_tokens = Vec::with_capacity(toks.len() - pos);
    let mut well_formed = true;
    for &t in &toks[pos..] {
        match tree.token_kind(t) {
            SyntaxKind::AMP => continue,
            SyntaxKind::NUMBER => match parse_float(&tree.token_text(t)) {
                Some(v) => {
                    coeffs.push(v);
                    coeff_tokens.push(t);
                }
                None => well_formed = false,
            },
            _ => well_formed = false,
        }
    }

    Some(Surface {
        card_index,
        id,
        id_token: id_tok,
        transform,
        transform_token,
        reflective,
        white,
        kind,
        coeffs,
        coeff_tokens,
        well_formed,
    })
}

/// Iterate all parseable surfaces in the model, in source order.
pub(crate) fn surfaces(tree: &GreenTree) -> impl Iterator<Item = Surface> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_surface(tree, i))
}

/// Minimal surface-header read for bulk edits: `(id_token, id, white)`.
/// Allocation-free — skips coefficient parsing entirely.
pub(crate) fn surface_id(tree: &GreenTree, card_index: usize) -> Option<(u32, i64, bool)> {
    let card = tree.cards()[card_index];
    if card.kind != SyntaxKind::SURFACE_CARD {
        return None;
    }
    let mut it = tree.card_content_tokens(&card);
    let mut tok = it.next()?;
    if tree.token_kind(tok) == SyntaxKind::STAR {
        tok = it.next()?;
    }
    if tree.token_kind(tok) != SyntaxKind::NUMBER {
        return None;
    }
    let text = tree.token_text(tok);
    let white = text.starts_with('+');
    let id = parse_int(&text)?;
    Some((tok, id, white))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crunchy_syntax::parse;

    fn model(surface_line: &str) -> GreenTree {
        // Minimal well-formed model with one surface.
        let src = format!("title\n1 0 -1\n\n{surface_line}\n\nm1 1001 1\n");
        parse(src).tree
    }

    #[test]
    fn simple_plane() {
        let t = model("113 PX -10");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.id, 113);
        assert_eq!(s.kind, SurfaceKind::Px);
        assert_eq!(s.coeffs, vec![-10.0]);
        assert!(s.transform.is_none());
        assert!(s.well_formed);
    }

    #[test]
    fn with_transform() {
        let t = model("5 3 CZ 2.5");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.id, 5);
        assert_eq!(s.transform, Some(3));
        assert_eq!(s.kind, SurfaceKind::Cz);
        assert_eq!(s.coeffs, vec![2.5]);
    }

    #[test]
    fn reflective_and_white() {
        let t = model("*7 PX 0");
        let s = surfaces(&t).next().unwrap();
        assert!(s.reflective);
        assert_eq!(s.id, 7);

        let t = model("+8 PY 0");
        let s = surfaces(&t).next().unwrap();
        assert!(s.white);
        assert_eq!(s.id, 8);
    }

    #[test]
    fn parenthesized_cylinder_mnemonic() {
        let t = model("9 C/X 0 0 3");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.kind, SurfaceKind::CParX);
        assert_eq!(s.coeffs, vec![0.0, 0.0, 3.0]);
    }

    #[test]
    fn mcnp_exponent_coefficient() {
        let t = model("10 SO 1.0-5");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.coeffs, vec![1.0e-5]);
    }

    #[test]
    fn gq_ten_coefficients() {
        let t = model("20 GQ 1 1 1 0 0 0 0 0 0 -25");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.kind, SurfaceKind::Gq);
        assert_eq!(s.coeffs.len(), 10);
    }

    #[test]
    fn ampersand_continuation_is_not_a_coefficient() {
        // An RPP split across lines with `&` (as seen in real models).
        let src = "title\n1 0 -1\n\n7 RPP 1 2 &\n     3 4 &\n     5 6\n\nm1 1001 1\n";
        let t = parse(src).tree;
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.kind, SurfaceKind::Rpp);
        assert_eq!(s.coeffs, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert!(s.well_formed, "& must not count as a coefficient");
    }

    #[test]
    fn unknown_mnemonic_preserved() {
        let t = model("30 ZZZ 1 2 3");
        let s = surfaces(&t).next().unwrap();
        assert_eq!(s.kind, SurfaceKind::Other("ZZZ".into()));
    }
}
