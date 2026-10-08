//! Typed projection of a surface card: `[*|+]id [transform] mnemonic coeffs...`

use migjorn_syntax::{Card, SyntaxKind};
use std::ops::Range;

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

/// The first coefficient that is not a number, as a token range and a message.
/// Coefficients are read up to it, so anything from it on would be lost; the
/// surface is not well formed instead. Shortcuts (`2R`, `3J`) are not expanded
/// and land here too.
pub(crate) fn coeff_problem(card: &Card, l: &SurfaceLayout) -> Option<(Range<usize>, String)> {
    let end = card.tokens().len();
    let mut i = l.coeffs_start;
    while let Some(k) = sig(card, i) {
        if k >= end {
            break;
        }
        if float_at(card, k).is_none() {
            return Some((k..k + 1, non_number_message(card.token_text(k))));
        }
        i = k + 1;
    }
    None
}

/// Message for a token that should be a number in a value list.
pub(crate) fn non_number_message(text: &str) -> String {
    let shortcut = text
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase();
    if !text.is_empty() && matches!(shortcut.as_str(), "r" | "i" | "j" | "m" | "ilog") {
        format!("shortcut `{text}` is not supported here")
    } else {
        format!("`{text}` is not a number")
    }
}

/// Coefficients up to the first token that is not a number; see
/// [`coeff_problem`] for what makes the surface not well formed.
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
