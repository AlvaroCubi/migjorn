use std::fmt;

use crate::parser_utils::OriginalBytes;
use thiserror::Error;

#[derive(Clone, Error)]
pub struct CardError {
    pub error: ParseError,
    pub original_bytes: OriginalBytes,
    pub position: usize,
}

impl CardError {
    pub fn new(error: ParseError, original_bytes: OriginalBytes, position: usize) -> Self {
        Self {
            error,
            original_bytes,
            position,
        }
    }
}

impl fmt::Display for CardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let number_of_bytes_to_show = 5.min(self.original_bytes.len() - self.position);
        write!(
            f,
            "Card parsing error: {}\nWord: {}\nCard: {}",
            self.error,
            String::from_utf8_lossy(
                &self.original_bytes.as_ref()
                    [self.position..self.position + number_of_bytes_to_show]
            ),
            String::from_utf8_lossy(self.original_bytes.as_ref()),
        )
    }
}

impl fmt::Debug for CardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Low-level, card-agnostic errors returned by primitive number parsers.
/// These are always mapped to a semantic [`ParseError`] at call sites and
/// never reach the user directly.
#[derive(Debug, Error)]
pub enum PrimitiveError {
    #[error("Unexpected end of input")]
    UnexpectedEnd,

    #[error("Invalid integer")]
    InvalidInteger,

    #[error("Invalid unsigned integer")]
    InvalidUnsigned,

    #[error("Invalid float")]
    InvalidFloat,
}

/// Semantic errors tied to a specific card field or domain.
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    // Cell card
    #[error("Invalid cell ID")]
    InvalidCellId,

    #[error("Invalid material number")]
    InvalidMaterial,

    #[error("Invalid density value")]
    InvalidDensity,

    #[error("Invalid geometry specification")]
    InvalidGeo,

    #[error("Unknown parameter")]
    UnknownParameter,

    // Surface card
    #[error("Invalid surface ID")]
    InvalidSurfaceId,

    #[error("Invalid transformation ID")]
    InvalidTransformId,

    #[error("Missing surface type")]
    MissingSurfType,

    #[error("Invalid surface type")]
    InvalidSurfType,

    #[error("Invalid coefficient")]
    InvalidCoefficient,

    // Data card (shared)
    #[error("Empty card")]
    EmptyCard,

    #[error("Missing card name")]
    MissingCardName,

    // Transformation card
    #[error("Invalid transformation number")]
    InvalidTransformNumber,

    // Material card
    #[error("Invalid material number")]
    InvalidMaterialNumber,

    #[error("Invalid isotope specification")]
    InvalidIsotope,

    #[error("Invalid fraction value")]
    InvalidFraction,

    #[error("Invalid library identifier")]
    InvalidLibrary,

    // Tally card
    #[error("Invalid tally number")]
    InvalidTallyNumber,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_error_display() {
        let error = PrimitiveError::UnexpectedEnd;
        assert_eq!(format!("{}", error), "Unexpected end of input");
    }

    #[test]
    fn test_parse_error_display() {
        let error = ParseError::InvalidCellId;
        assert_eq!(format!("{}", error), "Invalid cell ID");
    }

    #[test]
    fn test_card_error_display() {
        let card_bytes = b"1@WRONG 1 -2.7 -10".to_vec().into_boxed_slice();
        let card_bytes = OriginalBytes::Owned(card_bytes);
        let error = CardError::new(ParseError::InvalidCellId, card_bytes, 0);
        let expected = "\
Card parsing error: Invalid cell ID
Word: 1@WRO
Card: 1@WRONG 1 -2.7 -10";
        assert_eq!(format!("{}", error), expected);
    }
}
