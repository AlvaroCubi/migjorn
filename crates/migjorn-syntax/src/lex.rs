//! The per-card tokenizer.
//!
//! Lexing is scoped to a single card, which is what makes every edit local: a
//! card is re-lexed after it is spliced, and nothing else in the model is
//! touched. A card is tens of tokens, so this is microseconds.

use crate::kind::{CardKind, SyntaxKind};
use crate::lines::{is_comment_line, line_end, strip_eol};
use crate::token::Token;

/// Bytes that terminate a word and are themselves single-byte tokens (or, for
/// `$` and whitespace, start something handled specially).
#[inline]
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b':' | b'=' | b'$' | b'&' | b'#' | b'*'
    )
}

/// Classify a complete word.
///
/// A word starting with a digit is a `Number`, and so is one starting with a
/// sign or `.` *provided a digit follows somewhere in the word* — that keeps
/// `-1.0`, `+558.36` and `1e-3` numeric while a lone `-` (the dangling operator
/// in the malformed fixture) falls through to `Unknown`.
///
/// Data-array shortcuts (`3R`, `8I`, `2J`, `3M`, `4ILOG`) and ZAIDs
/// (`1001.31c`) also land here as `Number`; both are kept verbatim and never
/// expanded, which is required for losslessness.
fn classify_word(word: &[u8]) -> SyntaxKind {
    match word[0] {
        b'0'..=b'9' => SyntaxKind::Number,
        b'+' | b'-' | b'.' => {
            if word.iter().any(|b| b.is_ascii_digit()) {
                SyntaxKind::Number
            } else {
                SyntaxKind::Unknown
            }
        }
        b if b.is_ascii_alphabetic() || b == b'_' => SyntaxKind::Ident,
        _ => SyntaxKind::Unknown,
    }
}

/// Tokenize one card's text.
///
/// `Title`, `Message` and `Blank` cards are opaque — nothing projects from them,
/// so they get no tokens at all. This also avoids mis-reading a title that looks
/// like a comment (the reference model's title is the single byte `C`).
pub fn lex(text: &str, kind: CardKind) -> Vec<Token> {
    match kind {
        CardKind::Title | CardKind::Message | CardKind::Blank => return Vec::new(),
        _ => {}
    }

    let src = text.as_bytes();
    let mut tokens = Vec::new();
    let mut line_start = 0;

    while line_start < src.len() {
        let end = line_end(src, line_start);
        let line = &src[line_start..end];

        // A whole-line comment inside a card (an absorbed `c ...` line, or the
        // body of a standalone Comment card) is one token covering the line's
        // content, terminator excluded.
        if is_comment_line(line) {
            let body_len = strip_eol(line).len();
            tokens.push(Token::new(SyntaxKind::Comment, line_start, body_len));
            line_start = end;
            continue;
        }

        lex_line(src, line_start, end, &mut tokens);
        line_start = end;
    }

    tokens
}

fn lex_line(src: &[u8], from: usize, to: usize, tokens: &mut Vec<Token>) {
    let mut i = from;
    while i < to {
        let b = src[i];
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => {
                // Whitespace produces no token; the gap between spans is the
                // whitespace, and the card's own text holds the exact bytes.
                i += 1;
            }
            b'$' => {
                // Inline comment: runs to end of line, terminator excluded.
                let body_end = from + strip_eol(&src[from..to]).len();
                let end = body_end.max(i);
                tokens.push(Token::new(SyntaxKind::Comment, i, end - i));
                i = end;
            }
            b'&' => {
                tokens.push(Token::new(SyntaxKind::Ampersand, i, 1));
                i += 1;
            }
            b'(' | b')' | b':' | b'=' | b'*' | b'#' => {
                let kind = match b {
                    b'(' => SyntaxKind::LParen,
                    b')' => SyntaxKind::RParen,
                    b':' => SyntaxKind::Colon,
                    b'=' => SyntaxKind::Eq,
                    b'*' => SyntaxKind::Star,
                    _ => SyntaxKind::Hash,
                };
                tokens.push(Token::new(kind, i, 1));
                i += 1;
            }
            _ => {
                let start = i;
                while i < to && !is_delimiter(src[i]) {
                    i += 1;
                }
                let word = &src[start..i];
                tokens.push(Token::new(classify_word(word), start, word.len()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(SyntaxKind, &str)> {
        lex(text, CardKind::Cell)
            .into_iter()
            .map(|t| (t.kind, &text[t.range()]))
            .collect()
    }

    #[test]
    fn cell_card_with_inline_comment() {
        use SyntaxKind::*;
        assert_eq!(
            kinds("1 1 -1.0 -1 imp:n=1 $ fuel sphere\n"),
            vec![
                (Number, "1"),
                (Number, "1"),
                (Number, "-1.0"),
                (Number, "-1"),
                (Ident, "imp"),
                (Colon, ":"),
                (Ident, "n"),
                (Eq, "="),
                (Number, "1"),
                (Comment, "$ fuel sphere"),
            ]
        );
    }

    #[test]
    fn geometry_operators() {
        use SyntaxKind::*;
        assert_eq!(
            kinds("3 0 #1 #(2 -3) imp:n=1\n"),
            vec![
                (Number, "3"),
                (Number, "0"),
                (Hash, "#"),
                (Number, "1"),
                (Hash, "#"),
                (LParen, "("),
                (Number, "2"),
                (Number, "-3"),
                (RParen, ")"),
                (Ident, "imp"),
                (Colon, ":"),
                (Ident, "n"),
                (Eq, "="),
                (Number, "1"),
            ]
        );
    }

    #[test]
    fn boundary_prefixes_and_shortcuts() {
        use SyntaxKind::*;
        // `+3` keeps its prefix inside the number token, so a renumber that
        // rewrites the token can preserve the white-boundary marker.
        assert_eq!(
            kinds("+3  PY  0\n"),
            vec![(Number, "+3"), (Ident, "PY"), (Number, "0")]
        );
        // `*` is its own token, so `*2` is a reflective surface 2.
        assert_eq!(
            kinds("*2  PX  0\n"),
            vec![(Star, "*"), (Number, "2"), (Ident, "PX"), (Number, "0")]
        );
        // Shortcuts stay verbatim.
        assert_eq!(
            kinds("e0 1e-3 8I 1 2J 20\n"),
            vec![
                (Ident, "e0"),
                (Number, "1e-3"),
                (Number, "8I"),
                (Number, "1"),
                (Number, "2J"),
                (Number, "20"),
            ]
        );
    }

    #[test]
    fn absorbed_comment_lines_are_single_tokens() {
        use SyntaxKind::*;
        let text = "2 0 (1 : -2) -4\nc absorbed\n     imp:n=0\n";
        let got = kinds(text);
        assert!(got.contains(&(Comment, "c absorbed")));
        assert_eq!(got.last(), Some(&(Number, "0")));
    }

    #[test]
    fn malformed_words_are_unknown_not_a_panic() {
        use SyntaxKind::*;
        assert_eq!(kinds("2 0 1 -\n").last(), Some(&(Unknown, "-")));
        assert!(kinds("m1 1001 zzz\n").contains(&(Ident, "zzz")));
    }

    #[test]
    fn opaque_cards_have_no_tokens() {
        assert!(lex("C\r\n", CardKind::Title).is_empty());
        assert!(lex("\n", CardKind::Blank).is_empty());
    }
}
