use std::fmt::Display;

use crate::parser_utils::{
    CardError, OriginalBytes, ParseError, Spanned, approx_eq_f64, consume_word, parse_f64_fast,
    parse_spanned_u32, skip_whitespace,
};

#[derive(Debug, Clone)]
pub struct TransformationCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) transform_id: Spanned<u32>,
    pub(crate) starred: bool,
    pub(crate) coeffs: Vec<f64>,
}

impl PartialEq for TransformationCard {
    fn eq(&self, other: &Self) -> bool {
        self.transform_id.value == other.transform_id.value
            && self.starred == other.starred
            && self.coeffs.len() == other.coeffs.len()
            && self
                .coeffs
                .iter()
                .zip(other.coeffs.iter())
                .all(|(&a, &b)| approx_eq_f64(a, b))
    }
}

impl TransformationCard {
    pub fn transform_id(&self) -> u32 {
        self.transform_id.value
    }

    pub fn set_transform_id(&mut self, new_id: u32) {
        self.transform_id.value = new_id;
    }

    /// No setter as they are not spanned, written as they appear in the original bytes
    pub fn starred(&self) -> bool {
        self.starred
    }

    /// No setter as they are not spanned, written as they appear in the original bytes
    pub fn coeffs(&self) -> &[f64] {
        &self.coeffs
    }
}

impl TransformationCard {
    pub fn parse(
        original_bytes: OriginalBytes,
        pos: &mut usize,
    ) -> Result<TransformationCard, CardError> {
        let bytes = original_bytes.as_ref();
        let keyword = consume_word(bytes, pos).ok_or_else(|| {
            CardError::new(ParseError::MissingCardName, original_bytes.clone(), *pos)
        })?;
        let starred = keyword[0] == b'*';

        let transform_number = parse_spanned_u32(bytes, pos).map_err(|_| {
            CardError::new(
                ParseError::InvalidTransformNumber,
                original_bytes.clone(),
                *pos,
            )
        })?;

        let mut coeffs = Vec::new();
        while *pos < bytes.len() {
            skip_whitespace(bytes, pos);
            if *pos >= bytes.len() {
                break;
            }
            let coeff = parse_f64_fast(bytes, pos).map_err(|_| {
                CardError::new(ParseError::InvalidCoefficient, original_bytes.clone(), *pos)
            })?;
            coeffs.push(coeff);
        }

        Ok(TransformationCard {
            original_bytes,
            transform_id: transform_number,
            starred,
            coeffs,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let src = self.original_bytes.as_ref();
        let mut result = Vec::with_capacity(src.len());
        result.extend_from_slice(&src[..self.transform_id.span.0]);
        self.transform_id.write_into(&mut result);
        result.extend_from_slice(&src[self.transform_id.span.1..]);
        result
    }
}

impl TryFrom<OriginalBytes> for TransformationCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        TransformationCard::parse(bytes, &mut 0)
    }
}

impl TryFrom<&str> for TransformationCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        TransformationCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for TransformationCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransformationCard: transform_number={}, starred={}, coeffs=[{}]",
            self.transform_id.value,
            self.starred,
            self.coeffs
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Card, DataCard, parser_utils::OriginalBytes};

    #[test]
    fn test_parse_transform_card() {
        let card_bytes = OriginalBytes::from("TR58 1 2 3");
        let tr_card = TransformationCard::parse(card_bytes.clone(), &mut 0).unwrap();
        assert_eq!(tr_card.original_bytes, card_bytes);
        assert_eq!(tr_card.transform_id.value, 58);
        assert!(!tr_card.starred());
        assert_eq!(tr_card.coeffs(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_parse_starred_transform_card() {
        let card_bytes = OriginalBytes::from("C comment\n *TR58 1 2 3 4.1e-2 5 6.0 $ comment");
        let tr_card = TransformationCard::parse(card_bytes.clone(), &mut 0).unwrap();
        assert_eq!(tr_card.original_bytes, card_bytes);
        assert_eq!(tr_card.transform_id.value, 58);
        assert!(tr_card.starred());
        assert_eq!(tr_card.coeffs(), vec![1.0, 2.0, 3.0, 0.041, 5.0, 6.0]);
    }

    #[test]
    fn test_set_transform_id() {
        let mut data_card = DataCard::try_from("TR58 1 2 3").unwrap();
        let tr_card = data_card.as_transform_mut().unwrap();
        assert_eq!(tr_card.transform_id.value, 58);
        tr_card.set_transform_id(42);
        assert_eq!(tr_card.transform_id.value, 42);
        assert_eq!(data_card.updated_text(), "TR42 1 2 3");
    }

    #[test]
    fn tryfrom_str_roundtrip() {
        let input = "TR7 1.0 0.0 0.0 0.0 1.0 0.0 0.0 0.0 1.0";
        let card = TransformationCard::try_from(input).unwrap();
        assert_eq!(card.transform_id(), 7);
        assert_eq!(card.coeffs().len(), 9);
        assert_eq!(String::from_utf8(card.to_bytes()).unwrap(), input);
    }

    #[test]
    fn tryfrom_str_set_id_roundtrip() {
        let mut card = TransformationCard::try_from("TR58 1 2 3").unwrap();
        card.set_transform_id(10);
        assert_eq!(String::from_utf8(card.to_bytes()).unwrap(), "TR10 1 2 3");
    }
}
