use crate::parser_utils::*;
use std::sync::Arc;

/// Extract card boundaries in one pass and wrap each as a `OriginalBytes::Source`
/// pointing into the shared `source` buffer — zero per-card allocations.
pub fn get_cards_original_bytes(source: &Arc<[u8]>) -> Vec<OriginalBytes> {
    let bytes: &[u8] = source;
    let len = bytes.len();

    // The first line is always the title
    let title_end = memchr::memchr(b'\n', bytes).unwrap_or(len) + 1;

    let slices: Vec<_> = std::iter::once((0, title_end))
        .chain(get_slices_sequential(bytes, title_end, len))
        .collect();

    // Return the cards as `OriginalBytes::Source`
    slices
        .into_iter()
        .map(|(offset, card_len)| OriginalBytes::Source {
            source: source.clone(),
            offset,
            len: card_len,
        })
        .collect()
}

fn get_slices_sequential(bytes: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut cards: Vec<(usize, usize)> = Vec::new();

    // Line comments go before a card def
    let mut card_start = start;
    let mut line_start = start;
    let mut header_comment: Option<usize> = None;
    while line_start < end {
        // Find end of current line using SIMD-accelerated search
        let line_end = memchr::memchr(b'\n', &bytes[line_start..end])
            .map(|pos| line_start + pos)
            .unwrap_or(end);

        if is_continuation_line(&bytes[line_start..line_end]) {
            header_comment = None;
            line_start = line_end + 1;
            continue;
        }

        if is_line_comment(&bytes[line_start..line_end]) {
            if header_comment.is_none() {
                header_comment = Some(line_start);
            }
            line_start = line_end + 1;
            continue;
        }

        // Save previous card if exists
        if card_start < line_start {
            let end_card = header_comment.map_or(line_start, |h| h.min(line_start));
            if card_start < end_card {
                cards.push((card_start, end_card - card_start));
            }
        }

        // Start new card
        card_start = header_comment.unwrap_or(line_start);
        header_comment = None;
        line_start = line_end + 1;
    }

    // Push the last card if it exists and is not just trailing comments/whitespace
    if card_start < end {
        // Check if there's actual content (not just header comments at the end)
        // If we have a pending header_comment, it means this range ends with comments
        // that belong to a card outside this range - don't include them
        if header_comment.is_none() {
            cards.push((card_start, end - card_start));
        }
    }

    cards
}

#[inline]
fn is_continuation_line(line: &[u8]) -> bool {
    let len = line.len();

    // Check if there is a non whitespace char in the first 5 columns. If so it is not a cont.
    for i in 0..5.min(len) {
        if CHAR_CLASS[line[i] as usize] & WHITESPACE == 0 {
            return false;
        }
    }

    // All the first 5 columns (or less if line shorter) are whitespace.
    // If the line stops here it is not a cont.
    if len < 5 {
        return false;
    }

    // If the line is longer but all the chars are whitespace it is a blank line (not a cont.).
    for i in 5..len {
        if CHAR_CLASS[line[i] as usize] & WHITESPACE == 0 {
            return true;
        }
    }

    false
}

#[inline]
fn is_line_comment(line: &[u8]) -> bool {
    let len = line.len();

    // Line comments require the cC to appear in one of the first 5 colums
    for i in 0..5.min(len) {
        let class = CHAR_CLASS[line[i] as usize];
        if class & LINE_COMMENT != 0 {
            // If the cC is the last char of the line, it is a comment
            if i + 1 >= len {
                return true;
            }
            // If there is a keyword char right after the cC, it is not a comment (e.g. C/Z surf)
            if CHAR_CLASS[line[i + 1] as usize] & KEYWORD_CHAR != 0 {
                return false;
            }
            return true;
        } else if class & WHITESPACE == 0 {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &[u8] = b"\
Title of the file
C A line comment
c      another
100 2 -2.50   $ an inline comment
C Comment line in the middle
     (12 : 55) imp:N = 1
     $ an inline comment in a blank line
C This comment would belong to the card below
999 0 
       99
C Comment before blank line
  
C Surfaces
*99 SO 109.88
C Comment

SDEF pos 0 0 0
";

    #[test]
    fn test_get_slices_sequential() {
        let cards = get_slices_sequential(EXAMPLE, 0, EXAMPLE.len());

        fn s(data: &[u8], (offset, len): (usize, usize)) -> &[u8] {
            &data[offset..offset + len]
        }

        assert_eq!(s(EXAMPLE, cards[0]), b"Title of the file\n");
        assert_eq!(
            s(EXAMPLE, cards[1]),
            b"\
C A line comment
c      another
100 2 -2.50   $ an inline comment
C Comment line in the middle
     (12 : 55) imp:N = 1
     $ an inline comment in a blank line
"
        );
        assert_eq!(
            s(EXAMPLE, cards[2]),
            b"C This comment would belong to the card below\n999 0 \n       99\n"
        );
        assert_eq!(s(EXAMPLE, cards[3]), b"C Comment before blank line\n  \n");
        assert_eq!(s(EXAMPLE, cards[4]), b"C Surfaces\n*99 SO 109.88\n");
        assert_eq!(s(EXAMPLE, cards[5]), b"C Comment\n\n");
        assert_eq!(s(EXAMPLE, cards[6]), b"SDEF pos 0 0 0\n");
    }

    #[test]
    fn test_is_continuation_line() {
        // The lines that we pass to this function do not include the final '\n'
        assert!(is_continuation_line(b"     123"));
        assert!(is_continuation_line(b"             123 "));
        assert!(is_continuation_line(b"     $ an inline comment"));
        assert!(is_continuation_line(b"              $ an inline comment"));

        assert!(!is_continuation_line(b"123"));
        assert!(!is_continuation_line(b"    123"));
        assert!(!is_continuation_line(b""));
        assert!(!is_continuation_line(b"    "));
        assert!(!is_continuation_line(b"          "));
        assert!(!is_continuation_line(b"C"));
        assert!(!is_continuation_line(b"c"));
        assert!(!is_continuation_line(b"C   "));
        assert!(!is_continuation_line(b"  C"));
        assert!(!is_continuation_line(b"    C"));
        assert!(!is_continuation_line(b" $ an inline comment"));
    }

    #[test]
    fn test_is_line_comment() {
        // The lines that we pass to this function do not include the final '\n'
        assert!(is_line_comment(b"C"));
        assert!(is_line_comment(b" C    "));
        assert!(is_line_comment(b"    C 234 "));

        assert!(!is_line_comment(b" C/Z 23 "));
        assert!(!is_line_comment(b"12 C 23 "));
        assert!(!is_line_comment(b"     C\n"));
    }

    #[test]
    fn test_empty_and_small_files() {
        let empty: &[u8] = b"";
        let cards = get_slices_sequential(empty, 0, 0);
        assert_eq!(cards.len(), 0);

        let small = b"Single line\n";
        let cards = get_slices_sequential(small, 0, small.len());
        assert_eq!(cards.len(), 1);
        let (offset, len) = cards[0];
        assert_eq!(&small[offset..offset + len], small as &[u8]);
    }
}
