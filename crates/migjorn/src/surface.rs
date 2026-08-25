//! Typed projection of a surface card: `[*|+]id [transform] mnemonic coeffs...`

use migjorn_syntax::{Card, SyntaxKind};

use crate::scan::{float_at, int_at, kind_at, next, sig, text_at};

#[derive(Debug, Clone)]
pub(crate) struct SurfaceLayout {
    pub id: Option<i64>,
    pub id_tok: Option<usize>,
    /// Leading `*`: a reflective boundary.
    pub reflective: bool,
    /// Token index of the leading `*`, when present.
    pub star_tok: Option<usize>,
    /// Leading `+`: a white boundary. The `+` lives inside the id token, so
    /// renumbering must rewrite the token including its prefix.
    pub white: bool,
    /// Optional transform number before the mnemonic. Negative means periodic,
    /// and the sign must survive a renumber.
    pub transform: Option<i64>,
    pub transform_tok: Option<usize>,
    pub mnemonic_tok: Option<usize>,
    /// Token index of the first coefficient.
    pub coeffs_start: usize,
    pub well_formed: bool,
}

pub(crate) fn layout(card: &Card) -> SurfaceLayout {
    let end = card.tokens().len();
    let mut out = SurfaceLayout {
        id: None,
        id_tok: None,
        reflective: false,
        star_tok: None,
        white: false,
        transform: None,
        transform_tok: None,
        mnemonic_tok: None,
        coeffs_start: end,
        well_formed: false,
    };

    let Some(mut i) = sig(card, 0) else {
        return out;
    };
    if kind_at(card, i) == Some(SyntaxKind::Star) {
        out.reflective = true;
        out.star_tok = Some(i);
        match next(card, i) {
            Some(j) => i = j,
            None => return out,
        }
    }

    out.white = text_at(card, i).is_some_and(|t| t.starts_with('+'));
    out.id = int_at(card, i).filter(|&id| id > 0);
    out.id_tok = Some(i);

    let Some(j) = next(card, i) else {
        return out;
    };

    // A number here is a transform only if a mnemonic follows it; otherwise the
    // card is malformed and we would swallow the mnemonic slot.
    let mnemonic = if kind_at(card, j) == Some(SyntaxKind::Number) {
        match next(card, j).filter(|&k| kind_at(card, k) == Some(SyntaxKind::Ident)) {
            Some(k) => {
                out.transform = int_at(card, j);
                out.transform_tok = Some(j);
                Some(k)
            }
            None => None,
        }
    } else if kind_at(card, j) == Some(SyntaxKind::Ident) {
        Some(j)
    } else {
        None
    };

    out.mnemonic_tok = mnemonic;
    out.coeffs_start = mnemonic.and_then(|k| next(card, k)).unwrap_or(end);
    out.well_formed = out.id.is_some() && mnemonic.is_some() && out.coeffs_start < end;
    out
}

/// The surface's mnemonic, uppercased for comparison but returned as written.
pub(crate) fn mnemonic<'a>(card: &'a Card, l: &SurfaceLayout) -> Option<&'a str> {
    l.mnemonic_tok.and_then(|i| text_at(card, i))
}

/// Token indices of the surface's coefficients, in file order. The edit-side
/// companion to [`coeffs`]: it stops at the same first non-numeric token, so a
/// coefficient's read value and its editable token stay in lockstep.
pub(crate) fn coeff_tokens(card: &Card, l: &SurfaceLayout) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = l.coeffs_start;
    let end = card.tokens().len();
    while i < end {
        let Some(k) = sig(card, i) else { break };
        if k >= end {
            break;
        }
        if float_at(card, k).is_none() {
            break;
        }
        out.push(k);
        i = k + 1;
    }
    out
}

pub(crate) fn coeffs(card: &Card, l: &SurfaceLayout) -> Vec<f64> {
    let mut out = Vec::new();
    let mut i = l.coeffs_start;
    let end = card.tokens().len();
    while i < end {
        let Some(k) = sig(card, i) else { break };
        if k >= end {
            break;
        }
        match float_at(card, k) {
            Some(v) => out.push(v),
            None => break,
        }
        i = k + 1;
    }
    out
}
