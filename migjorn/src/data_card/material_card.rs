use std::fmt::Display;

use crate::parser_utils::{
    CardError, OriginalBytes, ParseError, Spanned, approx_eq_f64, consume_word, parse_f64_fast,
    parse_spanned_u32, parse_u32_fast, skip_whitespace,
};

#[derive(Debug, Clone)]
pub struct MaterialCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) material_id: Spanned<u32>,
    pub(crate) zaids: Vec<Zaid>,
}

impl PartialEq for MaterialCard {
    fn eq(&self, other: &Self) -> bool {
        self.material_id.value == other.material_id.value && self.zaids == other.zaids
    }
}

impl MaterialCard {
    pub fn material_id(&self) -> u32 {
        self.material_id.value
    }

    pub fn set_material_id(&mut self, new_id: u32) {
        self.material_id.value = new_id;
    }

    /// No setter as they are not spanned, written as they appear in the original bytes
    pub fn zaids(&self) -> &[Zaid] {
        &self.zaids
    }
}

#[derive(Debug, Clone)]
pub struct Zaid {
    pub isotope: u32,
    pub library: String,
    pub fraction: f64,
}

impl PartialEq for Zaid {
    fn eq(&self, other: &Self) -> bool {
        self.isotope == other.isotope
            && self.library == other.library
            && approx_eq_f64(self.fraction, other.fraction)
    }
}

impl MaterialCard {
    pub fn parse(
        original_bytes: OriginalBytes,
        pos: &mut usize,
    ) -> Result<MaterialCard, CardError> {
        let bytes = original_bytes.as_ref();
        let _keyword = consume_word(bytes, pos).ok_or_else(|| {
            CardError::new(ParseError::MissingCardName, original_bytes.clone(), *pos)
        })?;

        let material_number = parse_spanned_u32(bytes, pos).map_err(|_| {
            CardError::new(
                ParseError::InvalidMaterialNumber,
                original_bytes.clone(),
                *pos,
            )
        })?;

        let mut zaids = Vec::new();
        while *pos < original_bytes.len() {
            skip_whitespace(bytes, pos);
            if *pos >= original_bytes.len() {
                break;
            }

            let isotope = parse_u32_fast(bytes, pos).map_err(|_| {
                CardError::new(ParseError::InvalidIsotope, original_bytes.clone(), *pos)
            })?;

            if *pos + 4 > original_bytes.len() {
                return Err(CardError::new(
                    ParseError::InvalidLibrary,
                    original_bytes.clone(),
                    *pos,
                ));
            }
            *pos += 1; // Skip the '.' before the library extension
            let b = bytes;
            let library = String::from_utf8_lossy(&b[*pos..*pos + 3]).to_string();
            *pos += 3;

            let fraction = parse_f64_fast(bytes, pos).map_err(|_| {
                CardError::new(ParseError::InvalidFraction, original_bytes.clone(), *pos)
            })?;

            zaids.push(Zaid {
                isotope,
                library,
                fraction,
            });
        }

        Ok(MaterialCard {
            original_bytes,
            material_id: material_number,
            zaids,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let src = self.original_bytes.as_ref();
        let mut result = Vec::with_capacity(src.len());
        result.extend_from_slice(&src[..self.material_id.span.0]);
        self.material_id.write_into(&mut result);
        result.extend_from_slice(&src[self.material_id.span.1..]);
        result
    }
}

impl TryFrom<OriginalBytes> for MaterialCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        MaterialCard::parse(bytes, &mut 0)
    }
}

impl TryFrom<&str> for MaterialCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        MaterialCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for MaterialCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MaterialCard: material_number={}, zaids=[{:?}]",
            self.material_id.value, self.zaids
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Card, DataCard, parser_utils::OriginalBytes};

    #[test]
    fn test_parse_material_card() {
        let card_str = "M1 92235.80c 0.5 92238.80c 0.5";
        let card_bytes = OriginalBytes::from(card_str);
        let mut pos = 0;
        let material_card = MaterialCard::parse(card_bytes, &mut pos).unwrap();

        assert_eq!(material_card.material_id.value, 1);
        assert_eq!(material_card.zaids.len(), 2);
        assert_eq!(material_card.zaids[0].isotope, 92235);
        assert_eq!(material_card.zaids[0].library, "80c");
        assert_eq!(material_card.zaids[0].fraction, 0.5);
        assert_eq!(material_card.zaids[1].isotope, 92238);
        assert_eq!(material_card.zaids[1].library, "80c");
        assert_eq!(material_card.zaids[1].fraction, 0.5);
    }

    #[test]
    fn set_material_id() {
        let mut data_card = DataCard::try_from("M1 92235.80c 0.5").unwrap();
        let material_card = match &mut data_card {
            DataCard::Material(m) => m,
            _ => panic!("Expected a MaterialCard"),
        };
        assert_eq!(material_card.material_id(), 1);
        material_card.set_material_id(42);
        assert_eq!(material_card.material_id(), 42);
        assert_eq!(data_card.updated_text(), "M42 92235.80c 0.5");
    }

    #[test]
    fn get_material_zaids() {
        let data_card = DataCard::try_from("M1 92235.80c 0.5").unwrap();
        let material_card = data_card.as_material().unwrap();
        let zaids = material_card.zaids();
        assert_eq!(zaids.len(), 1);
        assert_eq!(zaids[0].isotope, 92235);
        assert_eq!(zaids[0].library, "80c");
        assert_eq!(zaids[0].fraction, 0.5);
    }

    #[test]
    fn tryfrom_str_roundtrip() {
        let input = "M42 92235.80c 0.5 92238.80c 0.5";
        let card = MaterialCard::try_from(input).unwrap();
        assert_eq!(card.material_id(), 42);
        assert_eq!(card.zaids().len(), 2);
        assert_eq!(String::from_utf8(card.to_bytes()).unwrap(), input);
    }

    #[test]
    fn tryfrom_str_set_id_roundtrip() {
        let mut card = MaterialCard::try_from("M1 92235.80c 0.5").unwrap();
        card.set_material_id(99);
        assert_eq!(
            String::from_utf8(card.to_bytes()).unwrap(),
            "M99 92235.80c 0.5"
        );
    }
}
