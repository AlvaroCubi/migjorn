//! Typed view of material (`Mn`) cards.
//!
//! A material card is `Mn ZAID1 frac1 ZAID2 frac2 … [keyword=value …]`. We
//! parse the ZAID/fraction pairs; keyword options (GAS, NLIB, …) are left in the
//! CST and simply end the entry list (they begin with an identifier, whereas
//! ZAIDs lex as numbers like `1001.31c`). This is the geometry-first scope: the
//! composition is modelled, exotic options are preserved but not interpreted.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::num::{parse_float, parse_int};

/// One `ZAID fraction` entry of a material.
#[derive(Debug, Clone)]
pub struct MaterialEntry {
    /// The ZAID as written, e.g. `1001.31c`, `92235.80c`, or `6000`.
    pub zaid: String,
    /// Token index of the ZAID (for in-place value edits).
    pub(crate) zaid_token: u32,
    /// Fraction (positive = atom fraction, negative = weight fraction).
    pub fraction: f64,
    /// Token index of the fraction (for in-place value edits).
    pub(crate) fraction_token: u32,
}

impl MaterialEntry {
    /// The ZA number (before the `.library` suffix), if numeric.
    pub fn za(&self) -> Option<i64> {
        let head = self.zaid.split('.').next().unwrap_or(&self.zaid);
        parse_int(head)
    }
}

/// A parsed material card.
#[derive(Debug, Clone)]
pub struct Material {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Material number (the `n` in `Mn`).
    pub id: i64,
    /// Token index of the id (embedded in the `Mn` mnemonic; for edits).
    pub(crate) id_token: u32,
    /// The `ZAID fraction` composition entries.
    pub entries: Vec<MaterialEntry>,
    /// False if the entries were not clean `ZAID fraction` pairs.
    pub well_formed: bool,
}

/// Parse the material card at `card_index`, or `None` if it is not an `Mn` card.
pub(crate) fn parse_material(tree: &GreenTree, card_index: usize) -> Option<Material> {
    let card = &tree.cards()[card_index];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let toks: Vec<u32> = tree
        .card_content_tokens(card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP)
        .collect();

    // Mnemonic must be `M<digits>` (not MT, MX, MODE, MPHYS, …).
    let name_tok = *toks.first()?;
    if tree.token_kind(name_tok) != SyntaxKind::IDENT {
        return None;
    }
    let name = tree.token_text(name_tok);
    let up = name.to_ascii_uppercase();
    let digits = up.strip_prefix('M')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id = parse_int(digits)?;

    // ZAID/fraction pairs until the first non-number (options) or end.
    let mut entries = Vec::new();
    let mut well_formed = true;
    let mut pos = 1;
    while let Some(&zt) = toks.get(pos) {
        if tree.token_kind(zt) != SyntaxKind::NUMBER {
            break; // options section begins
        }
        pos += 1;
        let Some(&ft) = toks.get(pos) else {
            well_formed = false;
            break;
        };
        if tree.token_kind(ft) != SyntaxKind::NUMBER {
            well_formed = false;
            break;
        }
        pos += 1;
        let fraction = parse_float(&tree.token_text(ft)).unwrap_or_else(|| {
            well_formed = false;
            f64::NAN
        });
        entries.push(MaterialEntry {
            zaid: tree.token_text(zt).into_owned(),
            zaid_token: zt,
            fraction,
            fraction_token: ft,
        });
    }

    Some(Material {
        card_index,
        id,
        id_token: name_tok,
        entries,
        well_formed,
    })
}

/// Iterate all `Mn` materials in the model, in source order.
pub(crate) fn materials(tree: &GreenTree) -> impl Iterator<Item = Material> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_material(tree, i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crunchy_syntax::parse;

    fn model(data_line: &str) -> GreenTree {
        let src = format!("title\n1 0 -1\n\n1 PX 0\n\n{data_line}\n");
        parse(src).tree
    }

    #[test]
    fn simple_material() {
        let t = model("m1 1001.31c 0.667 8016.31c 0.333");
        let m = materials(&t).next().unwrap();
        assert_eq!(m.id, 1);
        assert_eq!(m.entries.len(), 2);
        assert_eq!(m.entries[0].zaid, "1001.31c");
        assert_eq!(m.entries[0].fraction, 0.667);
        assert_eq!(m.entries[0].za(), Some(1001));
        assert!(m.well_formed);
    }

    #[test]
    fn weight_fractions_and_bare_zaid() {
        let t = model("m5 6000 -0.5 26000 -0.5");
        let m = materials(&t).next().unwrap();
        assert_eq!(m.entries[0].fraction, -0.5);
        assert_eq!(m.entries[1].za(), Some(26000));
    }

    #[test]
    fn options_end_the_entry_list() {
        let t = model("m2 1001 1 gas=1");
        let m = materials(&t).next().unwrap();
        assert_eq!(m.entries.len(), 1);
        assert!(m.well_formed);
    }

    #[test]
    fn mt_card_is_not_a_material() {
        let t = model("mt1 lwtr.10t");
        assert!(materials(&t).next().is_none());
    }
}
