use std::{fs, sync::Arc};

use crate::{
    DataCard, ModelError, ModelSection,
    parser_utils::{OriginalBytes, get_cards_original_bytes, is_blank_card},
};

/// Load data cards from a standalone data-cards file.
/// The first line is treated as the file title and skipped; cards are parsed
/// until the first blank line (MCNP section separator).
pub fn load_data_cards_file<P: AsRef<std::path::Path>>(
    file_path: P,
) -> Result<Vec<DataCard>, ModelError> {
    let bytes = fs::read(&file_path)?;
    let source: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    let mut card_bytes = get_cards_original_bytes(&source).into_iter();
    let title = card_bytes.next();
    let mut cards_iter = card_bytes.take_while(|card| !is_blank_card(card.as_ref()));

    let parse_card = |card: OriginalBytes| {
        DataCard::try_from(card).map_err(|e| ModelError::Parse {
            section: ModelSection::Data,
            error: e,
        })
    };

    // If the title is a comment line, prepend it to the first data card so
    // users can write a header comment describing the card or the file.
    let first_card = title
        .filter(|t| is_blank_card(t.as_ref()))
        .and_then(|title_bytes| {
            cards_iter.next().map(|first_card| {
                let mut combined = title_bytes.as_ref().to_vec();
                combined.extend_from_slice(first_card.as_ref());
                parse_card(OriginalBytes::from(combined))
            })
        });

    first_card
        .into_iter()
        .chain(cards_iter.map(parse_card))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Card;

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_material_and_transform_cards() {
        let mut f = NamedTempFile::new().unwrap();
        // The section parser treats the first line as the title card
        writeln!(f, "data cards file title").unwrap();
        writeln!(f, "M1 1001.80c 1.0").unwrap();
        writeln!(f, "TR7 1 0 0").unwrap();

        let cards = load_data_cards_file(f.path()).unwrap();
        assert_eq!(cards.len(), 2);
        assert!(cards[0].as_material().is_some());
        assert!(cards[1].as_transform().is_some());
        assert_eq!(cards[0].as_material().unwrap().material_id(), 1);
        assert_eq!(cards[1].as_transform().unwrap().transform_id(), 7);
    }

    #[test]
    fn missing_file_returns_io_error() {
        let result = load_data_cards_file("/nonexistent/path/cards.dat");
        assert!(matches!(result, Err(ModelError::Io(_))));
    }

    #[test]
    fn stops_at_blank_line() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "data cards file title").unwrap();
        writeln!(f, "M1 1001.80c 1.0").unwrap();
        writeln!(f).unwrap(); // blank line — cards after this are ignored
        writeln!(f, "TR7 1 0 0").unwrap();

        let cards = load_data_cards_file(f.path()).unwrap();
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn title_comment_is_combined_with_first_card() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "C data cards file title").unwrap();
        writeln!(f, "M1 1001.80c 1.0").unwrap();

        let cards = load_data_cards_file(f.path()).unwrap();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].as_material().is_some());
        assert_eq!(cards[0].as_material().unwrap().material_id(), 1);
        assert_eq!(
            cards[0].updated_text(),
            "C data cards file title\nM1 1001.80c 1.0\n"
        );
    }
}
