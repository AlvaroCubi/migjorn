use crate::parser_utils::OriginalBytes;

pub struct Sections {
    pub title: OriginalBytes,
    pub cells: Vec<OriginalBytes>,
    pub surfaces: Vec<OriginalBytes>,
    pub data_cards: Vec<OriginalBytes>,
}

pub fn divide_cards_slices_into_sections(card_slices: Vec<OriginalBytes>) -> Sections {
    if card_slices.is_empty() {
        panic!("No cards provided");
    }

    // Title is always the first card
    let mut cards_iter = card_slices.into_iter();
    let title = cards_iter.next().unwrap();
    let mut card_slices: Vec<OriginalBytes> = cards_iter.collect();

    let indices_of_blank_cards: Vec<usize> = card_slices
        .iter()
        .enumerate()
        .filter_map(|(i, card)| {
            if is_blank_card(card.as_ref()) {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    // Efficiently split into sections using slice ranges (zero-copy)
    let (cells, surfaces, data_cards) = match indices_of_blank_cards.as_slice() {
        [] => {
            // No blank cards - all cards go to cells section
            (card_slices, Vec::new(), Vec::new())
        }
        [first_blank] => {
            // One blank card - cells and surfaces, no data cards
            let surfaces = card_slices.split_off(first_blank + 1);
            card_slices.truncate(*first_blank); // Remove the blank card itself
            (card_slices, surfaces, Vec::new())
        }
        [first_blank, second_blank] => {
            // Two blank cards - all three sections
            let data_cards = card_slices.split_off(second_blank + 1);
            let mut surfaces = card_slices.split_off(first_blank + 1);
            surfaces.truncate(second_blank - first_blank - 1); // Remove second blank card
            card_slices.truncate(*first_blank); // Remove first blank card
            (card_slices, surfaces, data_cards)
        }
        [first_blank, second_blank, third_blank, ..] => {
            // Three or more blank cards - all three sections + ignore section
            let _ignored_cards = card_slices.split_off(third_blank + 1); // Ignore any cards after the third blank
            let mut data_cards = card_slices.split_off(second_blank + 1);
            let mut surfaces = card_slices.split_off(first_blank + 1);
            data_cards.truncate(third_blank - second_blank - 1); // Remove third blank card
            surfaces.truncate(second_blank - first_blank - 1); // Remove second blank card
            card_slices.truncate(*first_blank); // Remove first blank card
            (card_slices, surfaces, data_cards)
        }
    };

    Sections {
        title,
        cells,
        surfaces,
        data_cards,
    }
}

#[inline]
pub fn is_blank_card(card_slice: &[u8]) -> bool {
    // Manual byte scanning for maximum performance (avoid iterator/trim overhead)
    let mut i = 0;
    let len = card_slice.len();

    while i < len {
        // Skip leading whitespace (space, tab, carriage return)
        while i < len && matches!(card_slice[i], b' ' | b'\t' | b'\r') {
            i += 1;
        }

        if i >= len {
            break;
        }

        // Empty line - continue to next line
        if card_slice[i] == b'\n' {
            i += 1;
            continue;
        }

        // Comment line (starts with 'C' or 'c') - skip entire line
        if card_slice[i] == b'C' || card_slice[i] == b'c' {
            while i < len && card_slice[i] != b'\n' {
                i += 1;
            }
            if i < len {
                i += 1; // skip newline
            }
            continue;
        }

        // Found actual content - not a blank card
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_parsing() {
        let cards: Vec<OriginalBytes> = vec![
            OriginalBytes::from("Test Title"),
            OriginalBytes::from("1 0 -1"),
            OriginalBytes::from("2 0 1"),
            OriginalBytes::from("\n"),
            OriginalBytes::from("1 so 5.0"),
            OriginalBytes::from("2 px 10.0"),
            OriginalBytes::from("\n"),
            OriginalBytes::from("mode n"),
            OriginalBytes::from("nps 1000"),
            OriginalBytes::from("\n"),
            OriginalBytes::from("C This is a comment line that should be ignored"),
        ];

        let sections = divide_cards_slices_into_sections(cards);

        assert_eq!(sections.cells.len(), 2);
        assert_eq!(sections.surfaces.len(), 2);
        assert_eq!(sections.data_cards.len(), 2);

        // Verify actual content
        assert_eq!(sections.title.as_ref(), b"Test Title");
        assert_eq!(sections.cells[0].as_ref(), b"1 0 -1");
        assert_eq!(sections.surfaces[0].as_ref(), b"1 so 5.0");
        assert_eq!(sections.data_cards[0].as_ref(), b"mode n");
    }

    #[test]
    fn test_is_blank_card() {
        assert!(is_blank_card(b"\n"));
        assert!(is_blank_card(b""));
        assert!(is_blank_card(b"   \n"));
        assert!(is_blank_card(b"\r\n"));

        assert!(!is_blank_card(b"1 0 -1"));

        // Blank line with header comment lines should still be blank
        assert!(is_blank_card(b"C Header\n C Comment\n\n"));
    }
}
