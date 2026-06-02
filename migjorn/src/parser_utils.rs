mod character_class;
mod common_parsing_functions;
mod general_cards_parser;
mod number_parsing;
mod original_bytes;
mod parser_errors;
mod section_parser;
mod spanned;

pub use character_class::{CHAR_CLASS, DIGIT, GEOMETRY_OP, KEYWORD_CHAR, LINE_COMMENT, WHITESPACE};
pub use common_parsing_functions::{consume_word, peek_word, skip_whitespace};
pub use general_cards_parser::get_cards_original_bytes;
pub use number_parsing::{parse_f64_fast, parse_u32_fast};
pub use original_bytes::OriginalBytes;
pub use parser_errors::{CardError, ParseError};
pub use section_parser::divide_cards_slices_into_sections;
pub use section_parser::is_blank_card;
pub use spanned::{
    Span, Spanned, fmt_mcnp_sci, parse_spanned_f64, parse_spanned_i32, parse_spanned_u32,
    write_spanned,
};

/// Returns `true` if `a` and `b` are within a relative tolerance of 0.001% (1e-5).
/// When both values are zero the comparison is exact.
#[inline]
pub(crate) fn approx_eq_f64(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let max = a.abs().max(b.abs());
    (a - b).abs() / max <= 1e-5
}
