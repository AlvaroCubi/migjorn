use std::fmt::Display;

use crate::parser_utils::OriginalBytes;

#[derive(Debug, Clone)]
pub struct UnparsedCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) card_name: String,
}

impl PartialEq for UnparsedCard {
    fn eq(&self, other: &Self) -> bool {
        self.card_name == other.card_name
    }
}

impl UnparsedCard {
    pub fn card_name(&self) -> &str {
        &self.card_name
    }
}

impl Display for UnparsedCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Other: {}", self.card_name)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Card, DataCard};

    #[test]
    fn test_parse_other_card() {
        let data_card = DataCard::try_from("MISC some other card content").unwrap();
        let other_card = match &data_card {
            DataCard::Other(o) => o,
            _ => panic!("Expected an OtherCard"),
        };
        assert_eq!(data_card.updated_text(), "MISC some other card content");
        assert_eq!(other_card.card_name(), "MISC");
    }
}
