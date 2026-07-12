//! Typed view of coordinate-transformation (`TRn` / `*TRn`) data cards.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::num::{parse_float, parse_int};

/// A parsed `TRn` transformation card.
#[derive(Debug, Clone)]
pub struct Transform {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Transformation number.
    pub id: i64,
    /// Token index of the id (embedded in the `TRn` mnemonic; for edits).
    pub(crate) id_token: u32,
    /// Leading `*` — rotation entries are angles in degrees.
    pub degrees: bool,
    /// Displacement vector (missing components default to 0).
    pub displacement: [f64; 3],
    /// Token index of each displacement component that is present in the source
    /// (`None` for a component that defaulted to 0 and thus has no token). For
    /// in-place value edits.
    pub(crate) displacement_tokens: [Option<u32>; 3],
    /// Rotation entries as written (0, 3, 5, 6, or 9 per MCNP conventions).
    pub rotation: Vec<f64>,
    /// Token index of each rotation entry in `rotation` (parallel to it), for
    /// in-place value edits.
    pub(crate) rotation_tokens: Vec<u32>,
    /// False if a numeric entry failed to parse.
    pub well_formed: bool,
}

/// Parse the transform card at `card_index`, or `None` if it is not a `TRn`.
pub(crate) fn parse_transform(tree: &GreenTree, card_index: usize) -> Option<Transform> {
    let card = &tree.cards()[card_index];
    if card.kind != SyntaxKind::DATA_CARD {
        return None;
    }
    let toks: Vec<u32> = tree.card_content_tokens(card).collect();
    let mut pos = 0usize;

    let mut degrees = false;
    if toks.get(pos).map(|&i| tree.token_kind(i)) == Some(SyntaxKind::STAR) {
        degrees = true;
        pos += 1;
    }

    // Mnemonic `TR<number>`.
    let name_tok = *toks.get(pos)?;
    if tree.token_kind(name_tok) != SyntaxKind::IDENT {
        return None;
    }
    let name = tree.token_text(name_tok);
    let up = name.to_ascii_uppercase();
    let digits = up.strip_prefix("TR")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id = parse_int(digits)?;
    pos += 1;

    // Numeric entries (values plus their token indices, kept parallel).
    let mut nums = Vec::with_capacity(toks.len() - pos);
    let mut num_tokens = Vec::with_capacity(toks.len() - pos);
    let mut well_formed = true;
    for &t in &toks[pos..] {
        match tree.token_kind(t) {
            // `&` is a line-continuation marker, not a value.
            SyntaxKind::AMP => continue,
            SyntaxKind::NUMBER => match parse_float(&tree.token_text(t)) {
                Some(v) => {
                    nums.push(v);
                    num_tokens.push(t);
                }
                None => well_formed = false,
            },
            _ => well_formed = false,
        }
    }

    let mut displacement = [0.0; 3];
    let mut displacement_tokens = [None; 3];
    for i in 0..3 {
        if let Some(&v) = nums.get(i) {
            displacement[i] = v;
            displacement_tokens[i] = Some(num_tokens[i]);
        }
    }
    let (rotation, rotation_tokens) = if nums.len() > 3 {
        (nums[3..].to_vec(), num_tokens[3..].to_vec())
    } else {
        (Vec::new(), Vec::new())
    };

    Some(Transform {
        card_index,
        id,
        id_token: name_tok,
        degrees,
        displacement,
        displacement_tokens,
        rotation,
        rotation_tokens,
        well_formed,
    })
}

/// Iterate all `TRn` transforms in the model, in source order.
pub(crate) fn transforms(tree: &GreenTree) -> impl Iterator<Item = Transform> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_transform(tree, i))
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
    fn displacement_only() {
        let t = model("tr1 1.0 2.0 3.0");
        let tr = transforms(&t).next().unwrap();
        assert_eq!(tr.id, 1);
        assert!(!tr.degrees);
        assert_eq!(tr.displacement, [1.0, 2.0, 3.0]);
        assert!(tr.rotation.is_empty());
    }

    #[test]
    fn full_rotation_in_degrees() {
        let t = model("*TR5 0 0 0 90 90 0 90 0 90 0 90 90");
        let tr = transforms(&t).next().unwrap();
        assert_eq!(tr.id, 5);
        assert!(tr.degrees);
        assert_eq!(tr.rotation.len(), 9);
    }

    #[test]
    fn not_a_transform() {
        let t = model("m1 1001 1");
        assert!(transforms(&t).next().is_none());
    }
}
