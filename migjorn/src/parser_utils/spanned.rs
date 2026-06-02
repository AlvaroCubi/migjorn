use std::io::Write;

use super::{
    common_parsing_functions::skip_whitespace,
    number_parsing::{parse_f64_fast, parse_i32_fast, parse_u32_fast},
    parser_errors::PrimitiveError,
};

/// A byte-range within a card's `OriginalBytes` buffer.
///
/// `Span(start, end)` stores half-open indices `[start, end)`.
/// A zero-length span (`start == end`) represents a virtually inserted
/// value — it is written with a leading space but does not consume
/// any source bytes.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Span(pub usize, pub usize);

impl Span {
    /// A zero-length span anchored at `pos`, used for virtually inserted values.
    #[inline]
    pub fn empty_at(pos: usize) -> Self {
        Span(pos, pos)
    }

    /// Returns `true` when this span represents an insertion (start == end).
    #[inline]
    pub fn is_insertion(self) -> bool {
        self.0 == self.1
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    #[inline]
    pub fn new(value: T, span: Span) -> Self {
        Spanned { value, span }
    }
}

impl<T: std::fmt::Display> Spanned<T> {
    #[inline]
    pub fn write_into(&self, buf: &mut Vec<u8>) {
        write!(buf, "{}", self.value).unwrap();
    }
}

#[inline]
pub fn parse_spanned_i32(bytes: &[u8], pos: &mut usize) -> Result<Spanned<i32>, PrimitiveError> {
    skip_whitespace(bytes, pos);
    let start = *pos;
    let value = parse_i32_fast(bytes, pos).map_err(|_| PrimitiveError::InvalidInteger)?;
    Ok(Spanned::new(value, Span(start, *pos)))
}

#[inline]
pub fn parse_spanned_u32(bytes: &[u8], pos: &mut usize) -> Result<Spanned<u32>, PrimitiveError> {
    skip_whitespace(bytes, pos);
    let start = *pos;
    let value = parse_u32_fast(bytes, pos).map_err(|_| PrimitiveError::InvalidUnsigned)?;
    Ok(Spanned::new(value, Span(start, *pos)))
}

#[inline]
pub fn parse_spanned_f64(bytes: &[u8], pos: &mut usize) -> Result<Spanned<f64>, PrimitiveError> {
    skip_whitespace(bytes, pos);
    let start = *pos;
    let value = parse_f64_fast(bytes, pos).map_err(|_| PrimitiveError::InvalidFloat)?;
    Ok(Spanned::new(value, Span(start, *pos)))
}

#[inline]
pub fn write_spanned<T: std::fmt::Display>(
    source: &[u8],
    result: &mut Vec<u8>,
    pos: &mut usize,
    spanned: &Spanned<T>,
) {
    if spanned.span.is_insertion() {
        // Virtually inserted value (no original bytes): write with a leading space.
        // Do NOT advance pos so the following element can still copy the whitespace that was originally between the previous element and this one.
        result.push(b' ');
        spanned.write_into(result);
    } else {
        result.extend_from_slice(&source[*pos..spanned.span.0]);
        spanned.write_into(result);
        *pos = spanned.span.1;
    }
}

/// Format a float in MCNP scientific notation: `3.21543E-08` (5 decimal places, 2-digit exponent).
pub fn fmt_mcnp_sci(value: f64) -> String {
    let s = format!("{:.5E}", value);
    // Rust may produce a 1-digit exponent (e.g. "E-8"); pad it to 2 digits.
    if let Some(e_pos) = s.find('E') {
        let (mantissa, rest) = s.split_at(e_pos + 1); // includes 'E'
        let (sign, digits) = if rest.starts_with(['+', '-']) {
            rest.split_at(1)
        } else {
            ("+", rest)
        };
        if digits.len() < 2 {
            return format!("{mantissa}{sign}0{digits}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spanned_i32() {
        let bytes = b"C comment\n 12345 $ other comment";
        let mut pos = 0;
        let result = parse_spanned_i32(bytes, &mut pos).unwrap();
        assert_eq!(result.value, 12345);
        assert_eq!(result.span, Span(11, 16));

        let bytes = b"-999";
        let mut pos = 0;
        let result = parse_spanned_i32(bytes, &mut pos).unwrap();
        assert_eq!(result.value, -999);
        assert_eq!(result.span, Span(0, 4));
    }

    #[test]
    fn test_parse_spanned_f64() {
        let bytes = b"3.14159";
        let mut pos = 0;
        let result = parse_spanned_f64(bytes, &mut pos).unwrap();
        assert!((result.value - 3.14159).abs() < 1e-10);
        assert_eq!(result.span, Span(0, 7));

        let bytes = b"C comment\n    -2.7e-3 $ other comment";
        let mut pos = 0;
        let result = parse_spanned_f64(bytes, &mut pos).unwrap();
        assert!((result.value - (-2.7e-3)).abs() < 1e-10);
        assert_eq!(result.span, Span(14, 21));
    }
}
