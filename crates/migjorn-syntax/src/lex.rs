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
    // Whether we've reached the card's own first line yet: a leading `c ...`
    // line may be absorbed ahead of it, so that first line isn't always at
    // offset 0.
    let mut seen_content_line = false;

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

        // `FCn` / `SCn` (tally comment / source comment) cards are one line:
        // the mnemonic, then a free-text label for the rest of the line. MCNP
        // places no syntax on that label, so — like a `c ...` line — it gets
        // one opaque token instead of being run through the word lexer, which
        // would otherwise flag ordinary punctuation (`[#/cm^2/s]`) as unrecognized.
        if !seen_content_line && kind == CardKind::Data {
            if let Some(mnemonic_len) = free_text_mnemonic_len(line) {
                tokens.push(Token::new(SyntaxKind::Ident, line_start, mnemonic_len));
                let mut i = line_start + mnemonic_len;
                while i < end && matches!(src[i], b' ' | b'\t') {
                    i += 1;
                }
                let body_len = strip_eol(&src[i..end]).len();
                if body_len > 0 {
                    tokens.push(Token::new(SyntaxKind::Comment, i, body_len));
                }
                line_start = end;
                seen_content_line = true;
                continue;
            }
        }

        lex_line(src, line_start, end, &mut tokens);
        line_start = end;
        seen_content_line = true;
    }

    tokens
}

/// Length of a leading `FCn` / `SCn` mnemonic word at the start of `line`, if
/// the line begins with one; see [`is_free_text_mnemonic`].
fn free_text_mnemonic_len(line: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < line.len() && !is_delimiter(line[i]) {
        i += 1;
    }
    is_free_text_mnemonic(&line[..i]).then_some(i)
}

/// Whether `word` is an `FCn` / `SCn` tally/source comment mnemonic: `FC` or
/// `SC` (any case) followed by one or more digits.
fn is_free_text_mnemonic(word: &[u8]) -> bool {
    match word.get(0..2) {
        Some(prefix)
            if prefix.eq_ignore_ascii_case(b"fc") || prefix.eq_ignore_ascii_case(b"sc") =>
        {
            let rest = &word[2..];
            !rest.is_empty() && rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
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
        kinds_of(text, CardKind::Cell)
    }

    fn kinds_of(text: &str, kind: CardKind) -> Vec<(SyntaxKind, &str)> {
        lex(text, kind)
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
    fn fc_and_sc_cards_treat_the_label_as_free_text() {
        use SyntaxKind::*;
        assert_eq!(
            kinds_of(
                "FC202 Neutron flux per energy bin [#/cm^2/s]\n",
                CardKind::Data
            ),
            vec![
                (Ident, "FC202"),
                (Comment, "Neutron flux per energy bin [#/cm^2/s]"),
            ]
        );
        assert_eq!(
            kinds_of("sc1 a source comment: 50% [x]\n", CardKind::Data),
            vec![(Ident, "sc1"), (Comment, "a source comment: 50% [x]")]
        );
        // A bare mnemonic with no label still lexes cleanly.
        assert_eq!(kinds_of("FC14\n", CardKind::Data), vec![(Ident, "FC14")]);
        // `FCL` isn't `FC` + digits, so it lexes normally.
        assert_eq!(
            kinds_of("FCL124 0 0 0 1\n", CardKind::Data),
            vec![
                (Ident, "FCL124"),
                (Number, "0"),
                (Number, "0"),
                (Number, "0"),
                (Number, "1"),
            ]
        );
        // Only applies to Data cards — a coincidental match elsewhere lexes normally.
        assert_eq!(
            kinds_of(
                "FC202 Neutron flux per energy bin [#/cm^2/s]\n",
                CardKind::Cell
            ),
            vec![
                (Ident, "FC202"),
                (Ident, "Neutron"),
                (Ident, "flux"),
                (Ident, "per"),
                (Ident, "energy"),
                (Ident, "bin"),
                (Unknown, "["),
                (Hash, "#"),
                (Unknown, "/cm^2/s]"),
            ]
        );
        // A `c ...` line absorbed ahead of the card pushes the mnemonic past
        // offset 0; the free-text treatment still has to apply to it.
        assert_eq!(
            kinds_of(
                "c planes identify front, end\nFC202 flux [#/cm^2/s]\n",
                CardKind::Data
            ),
            vec![
                (Comment, "c planes identify front, end"),
                (Ident, "FC202"),
                (Comment, "flux [#/cm^2/s]"),
            ]
        );
    }

    #[test]
    fn opaque_cards_have_no_tokens() {
        assert!(lex("C\r\n", CardKind::Title).is_empty());
        assert!(lex("\n", CardKind::Blank).is_empty());
    }
}
