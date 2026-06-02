use std::fmt::Display;

pub use material_card::{MaterialCard, Zaid};
pub use tally_card::TallyCard;
pub use transformation_card::TransformationCard;
pub use unparsed_card::UnparsedCard;

use crate::Card;
use crate::parser_utils::{CardError, OriginalBytes, ParseError, peek_word};

mod material_card;
mod tally_card;
mod transformation_card;
mod unparsed_card;

#[derive(Debug, Clone)]
pub enum DataCard {
    Material(MaterialCard),
    Transform(TransformationCard),
    Tally(TallyCard),
    Other(UnparsedCard),
}

impl PartialEq for DataCard {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DataCard::Material(lhs), DataCard::Material(rhs)) => lhs == rhs,
            (DataCard::Transform(lhs), DataCard::Transform(rhs)) => lhs == rhs,
            (DataCard::Tally(lhs), DataCard::Tally(rhs)) => lhs == rhs,
            (DataCard::Other(lhs), DataCard::Other(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

impl Card for DataCard {
    fn original_bytes(&self) -> &[u8] {
        match self {
            DataCard::Material(m) => m.original_bytes.as_ref(),
            DataCard::Transform(t) => t.original_bytes.as_ref(),
            DataCard::Tally(t) => t.original_bytes.as_ref(),
            DataCard::Other(o) => o.original_bytes.as_ref(),
        }
    }

    fn updated_bytes(&self) -> Vec<u8> {
        match self {
            DataCard::Material(m) => m.to_bytes(),
            DataCard::Transform(t) => t.to_bytes(),
            DataCard::Tally(t) => t.to_bytes(),
            DataCard::Other(o) => o.original_bytes.as_ref().to_vec(),
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum DataCardId {
    Int(u32),
    String(String),
}

impl PartialOrd for DataCardId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DataCardId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (DataCardId::Int(a), DataCardId::Int(b)) => a.cmp(b),
            (DataCardId::String(a), DataCardId::String(b)) => a.cmp(b),
            // Int variants sort before String variants
            (DataCardId::Int(_), DataCardId::String(_)) => std::cmp::Ordering::Less,
            (DataCardId::String(_), DataCardId::Int(_)) => std::cmp::Ordering::Greater,
        }
    }
}

impl DataCard {
    pub fn card_id(&self) -> DataCardId {
        match self {
            DataCard::Material(m) => DataCardId::Int(m.material_id()),
            DataCard::Transform(t) => DataCardId::Int(t.transform_id()),
            DataCard::Tally(t) => DataCardId::Int(t.tally_id()),
            DataCard::Other(o) => DataCardId::String(o.card_name.clone()),
        }
    }

    pub fn as_material(&self) -> Option<&MaterialCard> {
        if let DataCard::Material(m) = self {
            Some(m)
        } else {
            None
        }
    }

    pub fn as_material_mut(&mut self) -> Option<&mut MaterialCard> {
        if let DataCard::Material(m) = self {
            Some(m)
        } else {
            None
        }
    }

    pub fn as_transform(&self) -> Option<&TransformationCard> {
        if let DataCard::Transform(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_transform_mut(&mut self) -> Option<&mut TransformationCard> {
        if let DataCard::Transform(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_tally(&self) -> Option<&TallyCard> {
        if let DataCard::Tally(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_tally_mut(&mut self) -> Option<&mut TallyCard> {
        if let DataCard::Tally(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_other(&self) -> Option<&UnparsedCard> {
        if let DataCard::Other(o) = self {
            Some(o)
        } else {
            None
        }
    }

    pub fn as_other_mut(&mut self) -> Option<&mut UnparsedCard> {
        if let DataCard::Other(o) = self {
            Some(o)
        } else {
            None
        }
    }
}

impl TryFrom<OriginalBytes> for DataCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        DataCard::parse(bytes)
    }
}

impl TryFrom<&str> for DataCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        DataCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for DataCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataCard::Material(m) => write!(f, "{}", m),
            DataCard::Transform(t) => write!(f, "{}", t),
            DataCard::Tally(t) => write!(f, "{}", t),
            DataCard::Other(o) => write!(f, "{}", o),
        }
    }
}

impl DataCard {
    fn parse(original_bytes: OriginalBytes) -> Result<DataCard, CardError> {
        let mut pos = 0;

        // Peek at the first word to determine card type before fully parsing
        let keyword = peek_word(original_bytes.as_ref(), 0)
            .ok_or_else(|| CardError::new(ParseError::MissingCardName, original_bytes.clone(), 0))?
            .to_ascii_uppercase();

        if keyword[0] == b'F' {
            return Ok(DataCard::Tally(TallyCard::parse(original_bytes, &mut pos)?));
        }

        let data_card = match keyword.as_slice() {
            b"M" => DataCard::Material(MaterialCard::parse(original_bytes, &mut pos)?),
            b"TR" => DataCard::Transform(TransformationCard::parse(original_bytes, &mut pos)?),
            b"*TR" => DataCard::Transform(TransformationCard::parse(original_bytes, &mut pos)?),
            _ => {
                let card_name = String::from_utf8_lossy(&keyword).to_string();
                DataCard::Other(UnparsedCard {
                    original_bytes,
                    card_name,
                })
            }
        };

        Ok(data_card)
    }
}
