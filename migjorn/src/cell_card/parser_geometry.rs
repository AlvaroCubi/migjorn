use crate::parser_utils::{
    CHAR_CLASS, GEOMETRY_OP, KEYWORD_CHAR, ParseError, Span, Spanned, parse_spanned_i32,
    skip_whitespace, write_spanned,
};
use std::fmt::Display;

#[derive(Debug, PartialEq, Clone)]
pub enum GeoElement {
    Surface(i32),
    Cell(u32),
    Operator(GeoOperator),
}

impl GeoElement {
    /// Parse a geometry token string (e.g. `"-10"`, `":"`, `"#40"`) into one or
    /// more [`GeoElement`]s. Note that `"#40"` yields two elements — a Complement
    /// operator followed by a Cell — and both should be inserted consecutively.
    pub fn parse_many(text: &str) -> Result<Vec<Self>, ParseError> {
        let bytes = text.as_bytes();
        let mut pos = 0;
        let elements_inner = parse_geometry(bytes, &mut pos)?;
        if elements_inner.is_empty() {
            return Err(ParseError::InvalidGeo);
        }
        Ok(elements_inner.into_iter().map(|e| e.value()).collect())
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum GeoElementSpanned {
    Surface(Spanned<i32>),
    Cell(Spanned<u32>),
    Operator(Spanned<GeoOperator>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum GeoOperator {
    LeftParen = b'(',
    RightParen = b')',
    Union = b':',
    Complement = b'#',
}

impl TryFrom<u8> for GeoOperator {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            b':' => Ok(GeoOperator::Union),
            b'(' => Ok(GeoOperator::LeftParen),
            b')' => Ok(GeoOperator::RightParen),
            b'#' => Ok(GeoOperator::Complement),
            _ => Err(()),
        }
    }
}

impl Display for GeoOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c = *self as u8 as char;
        write!(f, "{}", c)
    }
}

impl GeoElementSpanned {
    pub fn value(&self) -> GeoElement {
        match self {
            GeoElementSpanned::Surface(s) => GeoElement::Surface(s.value),
            GeoElementSpanned::Cell(c) => GeoElement::Cell(c.value),
            GeoElementSpanned::Operator(o) => GeoElement::Operator(o.value),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            GeoElementSpanned::Surface(s) => s.span,
            GeoElementSpanned::Cell(c) => c.span,
            GeoElementSpanned::Operator(o) => o.span,
        }
    }

    pub fn span_mut(&mut self) -> &mut Span {
        match self {
            GeoElementSpanned::Surface(s) => &mut s.span,
            GeoElementSpanned::Cell(c) => &mut c.span,
            GeoElementSpanned::Operator(o) => &mut o.span,
        }
    }

    pub fn write(
        &self,
        source: &[u8],
        result: &mut Vec<u8>,
        last_pos: &mut usize,
        prev: Option<&GeoElementSpanned>,
    ) {
        match self {
            GeoElementSpanned::Surface(s) => write_spanned(source, result, last_pos, s),
            GeoElementSpanned::Operator(o) => write_spanned(source, result, last_pos, o),
            GeoElementSpanned::Cell(c) => {
                // The complement operator (#) must be immediately adjacent to its
                // cell ID with no whitespace. For real spans this is guaranteed by
                // the source bytes; for virtually inserted elements we suppress the
                // leading space that write_spanned would normally add.
                let after_complement = matches!(
                    prev,
                    Some(GeoElementSpanned::Operator(op)) if op.value == GeoOperator::Complement
                );
                if after_complement && c.span.0 == c.span.1 {
                    c.write_into(result);
                } else {
                    write_spanned(source, result, last_pos, c);
                }
            }
        }
    }
}

pub fn parse_geometry(bytes: &[u8], pos: &mut usize) -> Result<Vec<GeoElementSpanned>, ParseError> {
    let mut elements = Vec::new();
    while *pos < bytes.len() {
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() {
            break;
        }

        let char_class = CHAR_CLASS[bytes[*pos] as usize];

        // Parse operator (match is optimized to a jump table)
        if char_class & GEOMETRY_OP != 0 {
            let operator = GeoOperator::try_from(bytes[*pos])
                .expect("CHAR_CLASS guarantees operator validity");
            elements.push(GeoElementSpanned::Operator(Spanned::new(
                operator,
                Span(*pos, *pos + 1),
            )));
            *pos += 1;
            continue;
        }

        // Check for keyword start
        if char_class & KEYWORD_CHAR != 0 {
            break;
        }

        // Parse signed integer (surface or cell ID)
        if let Ok(spanned_int) = parse_spanned_i32(bytes, pos) {
            let element = match elements.last() {
                Some(GeoElementSpanned::Operator(s)) if s.value == GeoOperator::Complement => {
                    GeoElementSpanned::Cell(Spanned::new(
                        spanned_int.value as u32,
                        spanned_int.span,
                    ))
                }
                _ => GeoElementSpanned::Surface(Spanned::new(spanned_int.value, spanned_int.span)),
            };
            elements.push(element);
            continue;
        }

        // Unknown character
        return Err(ParseError::InvalidGeo);
    }
    Ok(elements)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_geometry() {
        let bytes = b"C comment \n 10 -20 : (30 #40)  $ comment\n 50 IMP:N=1";
        let mut pos = 0;
        let geo = parse_geometry(bytes, &mut pos).unwrap();
        assert_eq!(geo.len(), 9);
        assert_eq!(
            geo[0],
            GeoElementSpanned::Surface(Spanned::new(10, Span(12, 14)))
        );
        assert_eq!(
            geo[1],
            GeoElementSpanned::Surface(Spanned::new(-20, Span(15, 18)))
        );
        assert_eq!(
            geo[2],
            GeoElementSpanned::Operator(Spanned::new(GeoOperator::Union, Span(19, 20)))
        );
        assert_eq!(
            geo[3],
            GeoElementSpanned::Operator(Spanned::new(GeoOperator::LeftParen, Span(21, 22)))
        );
        assert_eq!(
            geo[4],
            GeoElementSpanned::Surface(Spanned::new(30, Span(22, 24)))
        );
        assert_eq!(
            geo[5],
            GeoElementSpanned::Operator(Spanned::new(GeoOperator::Complement, Span(25, 26)))
        );
        assert_eq!(
            geo[6],
            GeoElementSpanned::Cell(Spanned::new(40, Span(26, 28)))
        );
        assert_eq!(
            geo[7],
            GeoElementSpanned::Operator(Spanned::new(GeoOperator::RightParen, Span(28, 29)))
        );
        assert_eq!(
            geo[8],
            GeoElementSpanned::Surface(Spanned::new(50, Span(42, 44)))
        );
    }

    #[test]
    fn to_geo_element() {
        let surface = GeoElementSpanned::Surface(Spanned::new(10, Span(0, 2)));
        let cell = GeoElementSpanned::Cell(Spanned::new(20, Span(3, 5)));
        let operator = GeoElementSpanned::Operator(Spanned::new(GeoOperator::Union, Span(6, 7)));

        assert_eq!(surface.value(), GeoElement::Surface(10));
        assert_eq!(cell.value(), GeoElement::Cell(20));
        assert_eq!(operator.value(), GeoElement::Operator(GeoOperator::Union));
    }
}
