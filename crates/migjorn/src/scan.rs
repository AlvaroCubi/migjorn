//! Low-level helpers for reading fields out of a card's tokens.
//!
//! Everything here is deliberately forgiving: a field that cannot be read comes
//! back as `None` rather than an error, and the caller reports the card as
//! not-well-formed. Malformed input must still project *something* and must
//! always round-trip.

use migjorn_syntax::{Card, SyntaxKind};

/// Index of the first non-trivia token at or after `i`.
pub(crate) fn sig(card: &Card, mut i: usize) -> Option<usize> {
    let tokens = card.tokens();
    while i < tokens.len() {
        if !tokens[i].is_trivia() {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Index of the next non-trivia token strictly after `i`.
#[inline]
pub(crate) fn next(card: &Card, i: usize) -> Option<usize> {
    sig(card, i + 1)
}

#[inline]
pub(crate) fn kind_at(card: &Card, i: usize) -> Option<SyntaxKind> {
    card.tokens().get(i).map(|t| t.kind)
}

#[inline]
pub(crate) fn text_at(card: &Card, i: usize) -> Option<&str> {
    if i < card.tokens().len() {
        Some(card.token_text(i))
    } else {
        None
    }
}

/// Parse a token as an integer, tolerating a leading `+`.
pub(crate) fn int_at(card: &Card, i: usize) -> Option<i64> {
    let text = text_at(card, i)?;
    if kind_at(card, i) != Some(SyntaxKind::Number) {
        return None;
    }
    parse_int(text)
}

pub(crate) fn parse_int(text: &str) -> Option<i64> {
    text.strip_prefix('+').unwrap_or(text).parse::<i64>().ok()
}

/// Parse a token as a float.
pub(crate) fn float_at(card: &Card, i: usize) -> Option<f64> {
    let text = text_at(card, i)?;
    if kind_at(card, i) != Some(SyntaxKind::Number) {
        return None;
    }
    parse_float(text)
}

/// Parse an MCNP float literal. MCNP accepts a Fortran-ish exponent form that
/// `f64::from_str` rejects: the `e` may be omitted when a sign follows the
/// mantissa directly, e.g. `1.0-5` means `1.0e-5` and `6.02+23` means `6.02e23`.
pub(crate) fn parse_float(text: &str) -> Option<f64> {
    let text = text.strip_prefix('+').unwrap_or(text);
    if let Ok(v) = text.parse::<f64>() {
        return Some(v);
    }
    // Look for a sign that introduces an implicit exponent: a `+`/`-` that is
    // not the leading character and is not already preceded by `e`/`E`.
    let bytes = text.as_bytes();
    for i in 1..bytes.len() {
        let b = bytes[i];
        if (b == b'+' || b == b'-') && !matches!(bytes[i - 1], b'e' | b'E') {
            let mut fixed = String::with_capacity(text.len() + 1);
            fixed.push_str(&text[..i]);
            fixed.push('e');
            fixed.push_str(&text[i..]);
            if let Ok(v) = fixed.parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// Split a data-card name into its alphabetic mnemonic and trailing number, e.g.
/// `m1` -> `("m", Some(1))`, `mode` -> `("mode", None)`, `f4` -> `("f", Some(4))`.
///
/// This is how `Mn` / `TRn` / `Fn` cards carry their id: the number is part of
/// the name token, not a separate field.
pub(crate) fn split_name(name: &str) -> (&str, Option<i64>) {
    let split = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len());
    let (alpha, digits) = name.split_at(split);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return (name, None);
    }
    (alpha, digits.parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_card_names_split_into_mnemonic_and_id() {
        assert_eq!(split_name("m1"), ("m", Some(1)));
        assert_eq!(split_name("mt1"), ("mt", Some(1)));
        assert_eq!(split_name("tr5"), ("tr", Some(5)));
        assert_eq!(split_name("f4"), ("f", Some(4)));
        assert_eq!(split_name("e0"), ("e", Some(0)));
        // no trailing number
        assert_eq!(split_name("mode"), ("mode", None));
        assert_eq!(split_name("sdef"), ("sdef", None));
        assert_eq!(split_name("print"), ("print", None));
        // digits must be a clean suffix
        assert_eq!(split_name("lwtr"), ("lwtr", None));
    }

    #[test]
    fn numbers_tolerate_a_leading_plus() {
        assert_eq!(parse_int("+558"), Some(558));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_float("+558.36"), Some(558.36));
        assert_eq!(parse_float("1e-3"), Some(0.001));
        // shortcuts are not numbers
        assert_eq!(parse_int("3R"), None);
    }

    #[test]
    fn mcnp_implicit_exponent_floats() {
        // MCNP lets the `e` be dropped when a sign directly follows the
        // mantissa: `1.0-5` means `1.0e-5`, not `1.0 - 5`.
        assert_eq!(parse_float("1.0-5"), Some(1.0e-5));
        assert_eq!(parse_float("6.02+23"), Some(6.02e23));
        assert_eq!(parse_float("-1.0-5"), Some(-1.0e-5));
        assert_eq!(parse_float("2.0+0"), Some(2.0));
        // a dangling sign with no digits after it is not a number
        assert_eq!(parse_float("1.0-"), None);
    }
}
