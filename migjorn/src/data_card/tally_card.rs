use std::fmt::Display;

use crate::{
    CardError,
    parser_utils::{OriginalBytes, ParseError, Spanned, consume_word, parse_spanned_u32},
};

#[derive(Debug, Clone)]
pub struct TallyCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) tally_id: Spanned<u32>,
}

impl PartialEq for TallyCard {
    fn eq(&self, other: &Self) -> bool {
        self.tally_id.value == other.tally_id.value
    }
}

impl TallyCard {
    pub fn tally_id(&self) -> u32 {
        self.tally_id.value
    }

    pub fn set_tally_id(&mut self, new_id: u32) {
        self.tally_id.value = new_id;
    }
}

impl TallyCard {
    pub fn parse(original_bytes: OriginalBytes, pos: &mut usize) -> Result<TallyCard, CardError> {
        let bytes = original_bytes.as_ref();
        consume_word(bytes, pos).ok_or_else(|| {
            CardError::new(ParseError::MissingCardName, original_bytes.clone(), *pos)
        })?;
        let tally_id = parse_spanned_u32(bytes, pos).map_err(|_| {
            CardError::new(ParseError::InvalidTallyNumber, original_bytes.clone(), *pos)
        })?;
        Ok(TallyCard {
            original_bytes,
            tally_id,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let src = self.original_bytes.as_ref();
        let mut result = Vec::with_capacity(src.len());
        result.extend_from_slice(&src[..self.tally_id.span.0]);
        self.tally_id.write_into(&mut result);
        result.extend_from_slice(&src[self.tally_id.span.1..]);
        result
    }
}

impl TryFrom<OriginalBytes> for TallyCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        TallyCard::parse(bytes, &mut 0)
    }
}

impl TryFrom<&str> for TallyCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        TallyCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for TallyCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tally card with id {}: {}",
            self.tally_id.value,
            String::from_utf8_lossy(&self.to_bytes())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Card, DataCard, parser_utils::OriginalBytes};

    #[test]
    fn test_parse_tally_card() {
        let card_bytes = OriginalBytes::from("F4:N 123");
        let tally_card = TallyCard::parse(card_bytes.clone(), &mut 0).unwrap();
        assert_eq!(tally_card.original_bytes, card_bytes);
        assert_eq!(tally_card.tally_id.value, 4);
    }

    #[test]
    fn test_parse_and_modify_several_tally_cards() {
        let data_card = DataCard::try_from("F1:N 10").unwrap();
        assert_eq!(data_card.updated_text(), "F1:N 10");
        let tally_card = match data_card {
            DataCard::Tally(t) => t,
            _ => panic!("Expected a TallyCard"),
        };
        assert_eq!(tally_card.tally_id(), 1);

        let mut data_card = DataCard::try_from("FMESH222:N 20").unwrap();
        let tally_card = match &mut data_card {
            DataCard::Tally(t) => t,
            _ => panic!("Expected a TallyCard"),
        };
        tally_card.set_tally_id(42);
        assert_eq!(tally_card.tally_id(), 42);
        assert_eq!(data_card.updated_text(), "FMESH42:N 20");

        let mut data_card = DataCard::try_from("FC4 This is a comment").unwrap();
        let tally_card = match &mut data_card {
            DataCard::Tally(t) => t,
            _ => panic!("Expected a TallyCard"),
        };
        tally_card.set_tally_id(99);
        assert_eq!(tally_card.tally_id(), 99);
        assert_eq!(data_card.updated_text(), "FC99 This is a comment");
    }

    #[test]
    fn tryfrom_str_roundtrip() {
        let input = "F4:N 10";
        let card = TallyCard::try_from(input).unwrap();
        assert_eq!(card.tally_id(), 4);
        assert_eq!(String::from_utf8(card.to_bytes()).unwrap(), input);
    }

    #[test]
    fn tryfrom_str_set_id_roundtrip() {
        let mut card = TallyCard::try_from("F4:N 10").unwrap();
        card.set_tally_id(8);
        assert_eq!(String::from_utf8(card.to_bytes()).unwrap(), "F8:N 10");
    }
}
