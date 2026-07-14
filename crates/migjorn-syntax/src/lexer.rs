//! A zero-copy, byte-oriented lexer for MCNP input.
//!
//! The lexer never allocates: it walks `src` once and reports each token to a
//! callback as `(kind, start, len)` byte offsets. Higher layers decide how to
//! group tokens (into a rowan tree, a token vec, etc.). All bytes of the input
//! are covered by exactly one token, which is what makes the CST lossless.

use crate::SyntaxKind::{self, *};

#[inline(always)]
fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Single-character tokens that always terminate a word.
#[inline(always)]
fn is_special(b: u8) -> bool {
    matches!(b, b'(' | b')' | b':' | b'#' | b'&' | b'=' | b'*')
}

#[inline(always)]
fn is_eol(b: u8) -> bool {
    b == b'\n' || b == b'\r'
}

/// True if `b` starts a `c`/`C` comment card, i.e. it is `c`/`C` and the
/// following byte (if any) is blank or ends the line.
#[inline(always)]
fn is_comment_head(src: &[u8], i: usize) -> bool {
    let b = src[i];
    if b != b'c' && b != b'C' {
        return false;
    }
    match src.get(i + 1) {
        None => true,
        Some(&nx) => is_ws(nx) || is_eol(nx),
    }
}

/// Lex `src`, invoking `emit(kind, start, len)` for every token in order.
///
/// Offsets are byte offsets into `src`. The input must be < 4 GiB so offsets
/// fit in `u32` (rowan's `TextSize`).
pub fn lex<F: FnMut(SyntaxKind, u32, u32)>(src: &[u8], mut emit: F) {
    let n = src.len();
    let mut i = 0usize;

    // Byte-order mark.
    if n >= 3 && src[0] == 0xEF && src[1] == 0xBB && src[2] == 0xBF {
        emit(BOM, 0, 3);
        i = 3;
    }

    // `at_line_start` gates column-sensitive constructs (comment cards live in
    // the name field, columns 1-5). It is set after every newline.
    let mut at_line_start = true;

    while i < n {
        let b = src[i];

        // --- line terminators ---
        if b == b'\n' {
            emit(NEWLINE, i as u32, 1);
            i += 1;
            at_line_start = true;
            continue;
        }
        if b == b'\r' {
            let start = i;
            i += 1;
            if i < n && src[i] == b'\n' {
                i += 1;
            }
            emit(NEWLINE, start as u32, (i - start) as u32);
            at_line_start = true;
            continue;
        }

        // --- whitespace (and comment-card detection after leading blanks) ---
        if is_ws(b) {
            let start = i;
            while i < n && is_ws(src[i]) {
                i += 1;
            }
            let leading = i - start;
            // A comment card may sit in columns 1-5, i.e. after <=4 blanks.
            if at_line_start && leading <= 4 && i < n && is_comment_head(src, i) {
                emit(WHITESPACE, start as u32, leading as u32);
                let cstart = i;
                while i < n && !is_eol(src[i]) {
                    i += 1;
                }
                emit(COMMENT_LINE, cstart as u32, (i - cstart) as u32);
                at_line_start = false;
                continue;
            }
            emit(WHITESPACE, start as u32, leading as u32);
            at_line_start = false;
            continue;
        }

        // --- comment card starting in column 1 ---
        if at_line_start && is_comment_head(src, i) {
            let cstart = i;
            while i < n && !is_eol(src[i]) {
                i += 1;
            }
            emit(COMMENT_LINE, cstart as u32, (i - cstart) as u32);
            at_line_start = false;
            continue;
        }
        at_line_start = false;

        // --- inline `$` comment ---
        if b == b'$' {
            let start = i;
            while i < n && !is_eol(src[i]) {
                i += 1;
            }
            emit(DOLLAR_COMMENT, start as u32, (i - start) as u32);
            continue;
        }

        // --- single-character punctuation ---
        let single = match b {
            b'(' => Some(L_PAREN),
            b')' => Some(R_PAREN),
            b':' => Some(COLON),
            b'#' => Some(HASH),
            b'&' => Some(AMP),
            b'=' => Some(EQ),
            b'*' => Some(STAR),
            _ => None,
        };
        if let Some(k) = single {
            emit(k, i as u32, 1);
            i += 1;
            continue;
        }

        // --- word: number or identifier ---
        let start = i;
        while i < n {
            let c = src[i];
            if is_ws(c) || is_eol(c) || c == b'$' || is_special(c) {
                break;
            }
            i += 1;
        }
        let kind = classify_word(&src[start..i]);
        emit(kind, start as u32, (i - start) as u32);
    }
}

/// Classify a word token as a number or an identifier. This is intentionally
/// permissive: exact float validity is a concern of the typed layer, not the
/// lexer. Anything that starts number-like is `NUMBER`.
#[inline]
fn classify_word(w: &[u8]) -> SyntaxKind {
    match w.first() {
        Some(c) if c.is_ascii_digit() => NUMBER,
        Some(b'+') | Some(b'-') | Some(b'.') => {
            if w.iter().any(u8::is_ascii_digit) {
                NUMBER
            } else {
                IDENT
            }
        }
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<(SyntaxKind, &str)> {
        let bytes = src.as_bytes();
        let mut out = Vec::new();
        lex(bytes, |k, s, l| {
            out.push((
                k,
                std::str::from_utf8(&bytes[s as usize..(s + l) as usize]).unwrap(),
            ))
        });
        out
    }

    /// Every token must tile the input exactly (lossless invariant).
    fn assert_lossless(src: &str) {
        let bytes = src.as_bytes();
        let mut pos = 0u32;
        lex(bytes, |_, s, l| {
            assert_eq!(s, pos, "gap/overlap in {src:?}");
            pos += l;
        });
        assert_eq!(pos as usize, bytes.len(), "did not cover all of {src:?}");
    }

    #[test]
    fn surface_card() {
        assert_eq!(
            toks("1 PX 3.5\n"),
            vec![
                (NUMBER, "1"),
                (WHITESPACE, " "),
                (IDENT, "PX"),
                (WHITESPACE, " "),
                (NUMBER, "3.5"),
                (NEWLINE, "\n"),
            ]
        );
    }

    #[test]
    fn comment_card_and_inline() {
        assert_eq!(
            toks("c hello\n1 SO 5 $ a sphere\n"),
            vec![
                (COMMENT_LINE, "c hello"),
                (NEWLINE, "\n"),
                (NUMBER, "1"),
                (WHITESPACE, " "),
                (IDENT, "SO"),
                (WHITESPACE, " "),
                (NUMBER, "5"),
                (WHITESPACE, " "),
                (DOLLAR_COMMENT, "$ a sphere"),
                (NEWLINE, "\n"),
            ]
        );
    }

    #[test]
    fn c_prefix_ident_is_not_a_comment() {
        // `cz` (a cylinder mnemonic) must not be swallowed as a comment card.
        assert_eq!(toks("1 CZ 4\n")[2], (IDENT, "CZ"));
    }

    #[test]
    fn mcnp_exponent_is_one_number() {
        assert_eq!(toks("1.0-5")[0], (NUMBER, "1.0-5"));
    }

    #[test]
    fn geometry_punctuation() {
        assert_eq!(
            toks("#(1:-2)"),
            vec![
                (HASH, "#"),
                (L_PAREN, "("),
                (NUMBER, "1"),
                (COLON, ":"),
                (NUMBER, "-2"),
                (R_PAREN, ")"),
            ]
        );
    }

    #[test]
    fn bom_is_tokenized() {
        assert_eq!(toks("\u{feff}c x")[0].0, BOM);
    }

    #[test]
    fn lossless_invariants() {
        assert_lossless("\u{feff}c title\n1 0 -1 imp:n=1 $ c\n\n 2 PY 3.0-4 &\n     4\n");
        assert_lossless("   c indented comment\n");
        assert_lossless("100 like 5 but mat=3\n");
    }
}
