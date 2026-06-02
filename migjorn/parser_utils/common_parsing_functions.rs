use crate::parser_utils::character_class::{CHAR_CLASS, KEYWORD_CHAR, WHITESPACE};

/// Skip whitespace and comments
#[inline]
pub fn skip_whitespace(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() {
        if CHAR_CLASS[bytes[*pos] as usize] & WHITESPACE != 0 {
            *pos += 1;
        } else if bytes[*pos] == b'$' || bytes[*pos] == b'&' {
            // Inline comment or line continuation
            go_to_end_of_line(bytes, pos);
        } else if bytes[*pos] == b'c' || bytes[*pos] == b'C' {
            // Check if c/C is in first 5 columns of line (columns 0-4)
            let mut line_start = *pos;
            while line_start > 0 && bytes[line_start - 1] != b'\n' {
                line_start -= 1;
            }
            let column = *pos - line_start;

            if column < 5
                && (*pos + 1 >= bytes.len()
                    || CHAR_CLASS[bytes[*pos + 1] as usize] & WHITESPACE != 0)
            {
                // c/C is a comment if in first 5 columns AND followed by whitespace (or end of data)
                go_to_end_of_line(bytes, pos);
            } else {
                break;
            }
        } else {
            break;
        }
    }
}

#[inline]
fn go_to_end_of_line(bytes: &[u8], pos: &mut usize) {
    // Move to \n or end
    *pos = memchr::memchr(b'\n', &bytes[*pos..])
        .map(|p| *pos + p)
        .unwrap_or(bytes.len());

    // Move past the newline character
    if *pos < bytes.len() {
        *pos += 1;
    }
}

#[inline]
pub fn consume_word<'a>(bytes: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    skip_whitespace(bytes, pos);
    if *pos >= bytes.len() {
        return None;
    }

    let start = *pos;
    while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & KEYWORD_CHAR != 0 {
        *pos += 1;
    }

    if start == *pos {
        None
    } else {
        Some(&bytes[start..*pos])
    }
}

#[inline]
pub fn peek_word(bytes: &[u8], pos: usize) -> Option<&[u8]> {
    let mut temp_pos = pos;
    consume_word(bytes, &mut temp_pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_to_end_of_line() {
        let bytes = b"Line 1\nLine 2\nLine 3";
        let mut pos = 0;
        go_to_end_of_line(bytes, &mut pos);
        assert_eq!(pos, 7); // After "Line 1\n"
        go_to_end_of_line(bytes, &mut pos);
        assert_eq!(pos, 14); // After "Line 2\n"
        go_to_end_of_line(bytes, &mut pos);
        assert_eq!(pos, bytes.len()); // After "Line 3" (end of data)
    }

    #[test]
    fn test_whitespace_skipping() {
        let bytes = b"   \t\n  42";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'4');
    }

    #[test]
    fn test_comment_detection() {
        // c at column 0 (start of line) - should be treated as comment
        let bytes = b"c this is a comment\n123";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'1');

        // c at column 1 with leading space - should be treated as comment
        let bytes = b" c this is a comment\n456";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'4');

        // c at column 4 - should be treated as comment (last valid column)
        let bytes = b"    c this is a comment\n789";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'7');

        // c at column 5 or beyond - should NOT be treated as comment
        let bytes = b"     c123";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'c'); // Should stop at 'c', not skip it

        // C/Z (surface type) - 'C' not followed by whitespace, not a comment
        let bytes = b"C/Z";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'C'); // Should not skip it

        // CZ (surface type) - 'C' not followed by whitespace, not a comment
        let bytes = b"CZ";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'C'); // Should not skip it

        // Inline comment with $ - should skip to next line
        let bytes = b"123 $ inline comment\n456";
        let mut pos = 3; // Start after "123"
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'4'); // Should skip whitespace, inline comment, and newline

        // $ at start of line
        let bytes = b"$ full line comment\n789";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(bytes[pos], b'7');

        // $ with no newline after (end of data)
        let bytes = b"   $ comment at end";
        let mut pos = 0;
        skip_whitespace(bytes, &mut pos);
        assert_eq!(pos, bytes.len()); // Should reach end of data
    }
}
