//! Typed projection of data cards: `Mn`, `TRn` / `*TRn`, and the generic
//! pass-through for everything else.

use migjorn_syntax::{Card, SyntaxKind};

use crate::scan::{float_at, kind_at, next, sig, split_name, text_at};

/// The head of any data card: an optional `*`, a mnemonic, an optional trailing
/// id baked into the mnemonic token, and an optional `:particle`.
#[derive(Debug, Clone)]
pub(crate) struct DataHead {
    pub starred: bool,
    /// Alphabetic part, lowercased (`m`, `mt`, `tr`, `f`, `sdef`).
    pub mnemonic: String,
    /// Trailing digits of the name token (`m1` -> 1, `sdef` -> None).
    pub number: Option<i64>,
    pub particle: Option<String>,
    pub name_tok: usize,
    /// Token index of the first value.
    pub values_start: usize,
}

pub(crate) fn head(card: &Card) -> Option<DataHead> {
    let mut i = sig(card, 0)?;
    let starred = kind_at(card, i) == Some(SyntaxKind::Star);
    if starred {
        i = next(card, i)?;
    }
    if kind_at(card, i) != Some(SyntaxKind::Ident) {
        return None;
    }
    let name = text_at(card, i)?;
    let (alpha, number) = split_name(name);
    let name_tok = i;

    let mut cursor = next(card, i);
    let mut particle = None;
    if cursor.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Colon)) {
        let after = next(card, cursor.unwrap());
        if after.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Ident)) {
            particle = Some(card.token_text(after.unwrap()).to_owned());
            cursor = next(card, after.unwrap());
        } else {
            cursor = after;
        }
    }

    Some(DataHead {
        starred,
        mnemonic: alpha.to_ascii_lowercase(),
        number,
        particle,
        name_tok,
        values_start: cursor.unwrap_or(card.tokens().len()),
    })
}

/// Is this an `Mn` material card? `mt1` / `mx1` / `mode` deliberately are not —
/// they reference a material or are unrelated, they do not define one.
pub(crate) fn material_id(head: &DataHead) -> Option<i64> {
    (head.mnemonic == "m").then_some(head.number).flatten()
}

/// Is this a `TRn` / `*TRn` transform card?
pub(crate) fn transform_id(head: &DataHead) -> Option<i64> {
    (head.mnemonic == "tr").then_some(head.number).flatten()
}

/// Material entries as `(zaid, fraction)` pairs.
///
/// The ZAID keeps its library suffix (`1001.31c`) verbatim. A positive fraction
/// is atomic, a negative one is by weight; both are returned as written.
pub(crate) fn material_entries(card: &Card, head: &DataHead) -> (Vec<(String, f64)>, bool) {
    let mut out = Vec::new();
    let mut well_formed = true;
    let end = card.tokens().len();
    let mut i = head.values_start;

    while i < end {
        let Some(zaid_tok) = sig(card, i) else { break };
        if zaid_tok >= end {
            break;
        }
        let zaid = card.token_text(zaid_tok).to_owned();
        let Some(frac_tok) = next(card, zaid_tok) else {
            well_formed = false;
            break;
        };
        match float_at(card, frac_tok) {
            Some(frac) => out.push((zaid, frac)),
            None => {
                well_formed = false;
                break;
            }
        }
        i = frac_tok + 1;
    }

    (out, well_formed)
}

/// Token indices of each material entry as `(zaid_tok, frac_tok)` pairs, in file
/// order. This is the edit-side companion to [`material_entries`]: the reader
/// wants the parsed values, an edit wants the token to rewrite. Stops at the
/// first entry it cannot read a fraction for, exactly as the reader does, so the
/// two always agree on which entries exist.
pub(crate) fn material_entry_tokens(card: &Card, head: &DataHead) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let end = card.tokens().len();
    let mut i = head.values_start;

    while i < end {
        let Some(zaid_tok) = sig(card, i) else { break };
        if zaid_tok >= end {
            break;
        }
        let Some(frac_tok) = next(card, zaid_tok) else {
            break;
        };
        if float_at(card, frac_tok).is_none() {
            break;
        }
        out.push((zaid_tok, frac_tok));
        i = frac_tok + 1;
    }

    out
}

/// Token index of the first value of a data card (past name/`:particle`), and the
/// index just past the last significant token. Used to replace a whole coefficient
/// list (transforms) in one splice.
pub(crate) fn values_span(card: &Card, from: usize) -> Option<(usize, usize)> {
    let end = card.tokens().len();
    let first = sig(card, from).filter(|&k| k < end)?;
    let mut last = first;
    let mut i = first;
    while i < end {
        let Some(k) = sig(card, i) else { break };
        if k >= end {
            break;
        }
        if float_at(card, k).is_none() {
            break;
        }
        last = k;
        i = k + 1;
    }
    Some((first, last))
}

/// All numeric values of a data card, trivia skipped. Used for transforms.
pub(crate) fn values(card: &Card, from: usize) -> Vec<f64> {
    let mut out = Vec::new();
    let end = card.tokens().len();
    let mut i = from;
    while i < end {
        let Some(k) = sig(card, i) else { break };
        if k >= end {
            break;
        }
        if let Some(v) = float_at(card, k) {
            out.push(v);
        }
        i = k + 1;
    }
    out
}
