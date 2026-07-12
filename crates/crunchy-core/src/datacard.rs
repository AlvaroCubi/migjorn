//! Generic view of any data card.
//!
//! In the geometry-first scope, data cards other than materials/transforms are
//! captured generically: mnemonic, optional particle designator, and the raw
//! token range. Nothing is lost (the CST is lossless); callers that need deep
//! semantics for a specific card can read its tokens.

use crunchy_syntax::{GreenTree, SyntaxKind};

/// A generically-parsed data card.
#[derive(Debug, Clone)]
pub struct DataCard {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Mnemonic, uppercased and including any trailing number (`SDEF`, `F4`,
    /// `M1`, `TR1`, `PHYS:N`→`PHYS`).
    pub name: String,
    /// Token index of the mnemonic.
    pub name_token: u32,
    /// Particle designator after a `:` (`n`, `p`, `n,p`), if present.
    pub particle: Option<String>,
    /// Leading `*` modifier (`*F`, `*TR`, `*C`).
    pub starred: bool,
}

/// Parse the data card at `card_index` generically, or `None` if it is not a
/// data card or has no mnemonic.
pub fn parse_data_card(tree: &GreenTree, card_index: usize) -> Option<DataCard> {
    let card = &tree.cards()[card_index];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let toks: Vec<u32> = tree
        .card_content_tokens(card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP)
        .collect();

    let mut pos = 0;
    let mut starred = false;
    if toks.get(pos).map(|&i| tree.token_kind(i)) == Some(SyntaxKind::STAR) {
        starred = true;
        pos += 1;
    }
    let name_token = *toks.get(pos)?;
    if tree.token_kind(name_token) != SyntaxKind::IDENT {
        return None;
    }
    let name = tree.token_text(name_token).to_ascii_uppercase();
    pos += 1;

    // Optional `:particle` designator.
    let mut particle = None;
    if toks.get(pos).map(|&i| tree.token_kind(i)) == Some(SyntaxKind::COLON) {
        pos += 1;
        if let Some(&pt) = toks.get(pos) {
            if tree.token_kind(pt) == SyntaxKind::IDENT {
                particle = Some(tree.token_text(pt).into_owned());
            }
        }
    }

    Some(DataCard {
        card_index,
        name,
        name_token,
        particle,
        starred,
    })
}

/// Iterate all data cards generically, in source order.
pub fn data_cards(tree: &GreenTree) -> impl Iterator<Item = DataCard> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_data_card(tree, i))
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
    fn plain_mnemonic() {
        let t = model("sdef pos=0 0 0 erg=14");
        let d = data_cards(&t).next().unwrap();
        assert_eq!(d.name, "SDEF");
        assert!(d.particle.is_none());
        assert!(!d.starred);
    }

    #[test]
    fn tally_with_particle() {
        let t = model("f4:n 1 2 3");
        let d = data_cards(&t).next().unwrap();
        assert_eq!(d.name, "F4");
        assert_eq!(d.particle.as_deref(), Some("n"));
    }

    #[test]
    fn starred_tally() {
        let t = model("*f1:p 5");
        let d = data_cards(&t).next().unwrap();
        assert!(d.starred);
        assert_eq!(d.name, "F1");
        assert_eq!(d.particle.as_deref(), Some("p"));
    }
}
