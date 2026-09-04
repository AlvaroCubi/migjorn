//! Line classification — the rules that decide where one card ends and the next
//! begins. Everything here operates on raw bytes and treats a "line" as its
//! content *including* its terminator, so card text is a contiguous byte range
//! and emission is a plain concatenation.

/// UTF-8 byte order mark.
pub(crate) const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[inline]
pub(crate) fn is_space(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Index just past the end of the line starting at `from` (i.e. past its `\n`,
/// or `src.len()` for a final line with no terminator).
#[inline]
pub(crate) fn line_end(src: &[u8], from: usize) -> usize {
    match memchr::memchr(b'\n', &src[from..]) {
        Some(i) => from + i + 1,
        None => src.len(),
    }
}

/// The line's bytes with its `\r\n` / `\n` terminator removed.
#[inline]
pub(crate) fn strip_eol(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

/// A blank line: empty or all whitespace. Blank lines delimit the three blocks,
/// so this is load-bearing structure, not trivia to be skipped.
#[inline]
pub(crate) fn is_blank(line: &[u8]) -> bool {
    strip_eol(line).iter().all(|&b| is_space(b))
}

/// Locate the first non-blank byte of a line as a **1-based column**, expanding
/// tabs to the next multiple of 8 the way MCNP does.
///
/// Both the comment rule and the continuation rule are stated in columns, and
/// real files do use tabs for indentation, so counting bytes instead of columns
/// silently misclassifies them.
fn first_content_column(body: &[u8]) -> Option<(usize, usize)> {
    let mut col = 1usize;
    for (i, &b) in body.iter().enumerate() {
        match b {
            b' ' => col += 1,
            b'\t' => col = (col - 1) / 8 * 8 + 9,
            _ => return Some((col, i)),
        }
    }
    None
}

/// A full-line comment: the first non-blank byte falls within **columns 1-5**
/// and is `c`/`C`, followed by whitespace or end of line.
///
/// The column bound matters. `     c foo` has its `c` in column 6, which makes
/// it a *blank-column continuation of the previous card*, not a comment — and
/// getting that backwards silently corrupts multi-line cards.
pub(crate) fn is_comment_line(line: &[u8]) -> bool {
    let body = strip_eol(line);
    let Some((col, i)) = first_content_column(body) else {
        return false; // blank
    };
    if col > 5 {
        return false;
    }
    if body[i] != b'c' && body[i] != b'C' {
        return false;
    }
    match body.get(i + 1) {
        None => true,
        Some(&b) => is_space(b),
    }
}

/// A continuation by the blank-column mechanism: a non-blank line whose content
/// starts beyond column 5. This is by far the dominant continuation form in real
/// files (5.19M occurrences in the 380 MB reference model, vs. 2 uses of `&`).
pub(crate) fn is_blank_col_continuation(line: &[u8]) -> bool {
    match first_content_column(strip_eol(line)) {
        Some((col, _)) => col > 5,
        None => false, // blank line
    }
}

/// Whether a content line ends with the `&` continuation marker.
///
/// The `&` must be found *after stripping any `$` inline comment* — `1 2 & $ ok`
/// continues, and `1 2 $ & not a continuation` does not.
pub(crate) fn ends_with_ampersand(line: &[u8]) -> bool {
    let mut body = strip_eol(line);
    if let Some(i) = memchr::memchr(b'$', body) {
        body = &body[..i];
    }
    while let Some((&last, rest)) = body.split_last() {
        if is_space(last) {
            body = rest;
        } else {
            return last == b'&';
        }
    }
    false
}

/// Does the file open with a `MESSAGE:` block? Checked after any BOM.
pub(crate) fn starts_with_message(src: &[u8]) -> bool {
    let src = src.strip_prefix(BOM).unwrap_or(src);
    let end = line_end(src, 0);
    let body = strip_eol(&src[..end]);
    let body = match body.iter().position(|&b| !is_space(b)) {
        Some(i) => &body[i..],
        None => return false,
    };
    body.len() >= 8 && body[..8].eq_ignore_ascii_case(b"MESSAGE:")
}

/// Byte offsets of every blank line in `src`, as `(start, end)` pairs.
///
/// Blank lines fully determine the block structure (a blank line ends the block
/// it follows, so no blank line can occur *inside* a block). The reference model
/// has exactly two in 377 MB, which is why they are useless as parallel-chunk
/// split points but perfect as a cheap block index.
pub(crate) fn find_blank_lines(src: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < src.len() {
        let end = line_end(src, pos);
        if is_blank(&src[pos..end]) {
            out.push((pos, end));
        }
        pos = end;
    }
    out
}

/// Scan backwards from `at` (a line start) to the previous *content* line,
/// skipping comment lines. Returns `None` if a blank line or the start of the
/// slice is reached first — in both cases there is no card to continue.
fn prev_content_line(src: &[u8], region_start: usize, at: usize) -> Option<&[u8]> {
    let mut end = at;
    while end > region_start {
        // Find the start of the line ending at `end`.
        let prev_nl = memchr::memrchr(b'\n', &src[region_start..end - 1]);
        let start = match prev_nl {
            Some(i) => region_start + i + 1,
            None => region_start,
        };
        let line = &src[start..end];
        if is_blank(line) {
            return None;
        }
        if !is_comment_line(line) {
            return Some(line);
        }
        end = start;
    }
    None
}

/// Snap `from` forward to the next position that genuinely starts a new card,
/// including any header comment run that will be absorbed into it.
///
/// Used to make parallel chunk boundaries safe: a chunk may only begin where the
/// sequential segmenter would also have begun a card. A comment run is never
/// split from the card it belongs to, so a boundary lands at the start of a
/// comment run precisely when that run is *not* absorbed backward (into a
/// continuation) — in which case it will be absorbed forward instead.
pub(crate) fn find_card_start(
    src: &[u8],
    region_start: usize,
    region_end: usize,
    from: usize,
) -> usize {
    // `from` is an arbitrary byte offset, so start at the next whole line.
    let mut pos = if from <= region_start {
        region_start
    } else {
        line_end(src, from).min(region_end)
    };
    // `pos` may land inside a comment run, or on the card line right after
    // one — both split the run from the card it belongs to, because the
    // forward scan below only ever sees lines at or after `pos`. Back up
    // over any contiguous comment lines immediately preceding `pos` first,
    // so the forward scan starts from the run's true beginning and can
    // correctly decide whether it is absorbed backward or forward.
    while pos > region_start {
        let prev_start = match memchr::memrchr(b'\n', &src[region_start..pos - 1]) {
            Some(i) => region_start + i + 1,
            None => region_start,
        };
        if !is_comment_line(&src[prev_start..pos]) {
            break;
        }
        pos = prev_start;
    }
    // Start of a not-yet-resolved comment run: a candidate boundary, unless the
    // next non-comment line turns out to continue whatever precedes it, in
    // which case the run is absorbed backward instead and is not a boundary.
    let mut comment_run_start: Option<usize> = None;
    while pos < region_end {
        let end = line_end(src, pos).min(region_end);
        let line = &src[pos..end];

        if is_blank(line) {
            comment_run_start = None;
            pos = end;
            continue;
        }
        if is_comment_line(line) {
            comment_run_start.get_or_insert(pos);
            pos = end;
            continue;
        }
        let continues = is_blank_col_continuation(line)
            || prev_content_line(src, region_start, pos).is_some_and(ends_with_ampersand);
        if continues {
            comment_run_start = None;
            pos = end;
            continue;
        }
        return comment_run_start.unwrap_or(pos);
    }
    region_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_lines_respect_the_five_column_rule() {
        assert!(is_comment_line(b"c a comment\n"));
        assert!(is_comment_line(b"C a comment\r\n"));
        assert!(is_comment_line(b"    c indented but still column 5\n"));
        assert!(is_comment_line(b"c\n"));
        assert!(is_comment_line(b"c"));
        // column 6 -> a continuation, not a comment
        assert!(!is_comment_line(
            b"     c this continues the previous card\n"
        ));
        // `c` must be followed by whitespace or EOL
        assert!(!is_comment_line(b"cx 1 2 3\n"));
        assert!(!is_blank(b"c\n"));
    }

    #[test]
    fn blank_column_continuations() {
        assert!(is_blank_col_continuation(b"     13 -14 imp:n=1\n"));
        // a leading tab reaches column 9, well past the 5-column boundary
        assert!(is_blank_col_continuation(b"\t 1 2\n"));
        assert!(!is_blank_col_continuation(b"   4 -5 imp:n=1\n")); // column 4
        assert!(!is_blank_col_continuation(b"    4 -5\n")); // column 5, still a card
        assert!(!is_blank_col_continuation(b"        \n")); // blank line
        assert!(!is_blank_col_continuation(b"1 0 -1\n"));
    }

    #[test]
    fn tabs_expand_to_columns_for_the_comment_rule() {
        // A tab lands on column 9, so this `c` is past the comment columns and
        // the line continues the previous card instead.
        assert!(!is_comment_line(b"\tc not a comment\n"));
        assert!(is_comment_line(b"c real comment\n"));
    }

    #[test]
    fn ampersand_is_read_past_inline_comments() {
        assert!(ends_with_ampersand(b"10 0 -1 2 -3 &\n"));
        assert!(ends_with_ampersand(b"10 0 -1 &   $ trailing note\r\n"));
        assert!(!ends_with_ampersand(b"10 0 -1 $ & not a continuation\n"));
        assert!(!ends_with_ampersand(b"10 0 -1\n"));
    }

    #[test]
    fn message_detection() {
        assert!(starts_with_message(b"MESSAGE: outp=o\n"));
        assert!(starts_with_message(b"\xEF\xBB\xBFmessage: outp=o\n"));
        assert!(!starts_with_message(b"Minimal valid model\n"));
    }
}
