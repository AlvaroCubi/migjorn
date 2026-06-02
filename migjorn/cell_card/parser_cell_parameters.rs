use std::io::Write;

use crate::parser_utils::{
    ParseError, Span, Spanned, approx_eq_f64, consume_word, fmt_mcnp_sci, parse_f64_fast,
    parse_u32_fast, peek_word, skip_whitespace,
};

#[derive(Debug, Clone)]
pub struct CellParam {
    pub param_type: ParamType,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub struct FillData {
    pub starred: bool,
    pub universe: u32,
    pub transform: Option<u32>,
    pub coeffs: Option<Vec<f64>>,
}

impl PartialEq for FillData {
    fn eq(&self, other: &Self) -> bool {
        use crate::parser_utils::approx_eq_f64;
        self.starred == other.starred
            && self.universe == other.universe
            && self.transform == other.transform
            && match (&self.coeffs, &other.coeffs) {
                (Some(a), Some(b)) => {
                    a.len() == b.len() && a.iter().zip(b.iter()).all(|(&x, &y)| approx_eq_f64(x, y))
                }
                (None, None) => true,
                _ => false,
            }
    }
}

/// Cell parameters (IMP, VOL, U, FILL, etc.)
#[derive(Debug, Clone)]
pub enum ParamType {
    ImpN(f64),
    ImpP(f64),
    Fill(FillData),
    U(u32),
    Vol(f64),
    Tmp(f64),
    Lat(u32),
    Other(String),
}

impl PartialEq for ParamType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ParamType::ImpN(a), ParamType::ImpN(b)) => approx_eq_f64(*a, *b),
            (ParamType::ImpP(a), ParamType::ImpP(b)) => approx_eq_f64(*a, *b),
            (ParamType::Vol(a), ParamType::Vol(b)) => approx_eq_f64(*a, *b),
            (ParamType::Tmp(a), ParamType::Tmp(b)) => approx_eq_f64(*a, *b),
            (ParamType::Fill(a), ParamType::Fill(b)) => a == b,
            (ParamType::U(a), ParamType::U(b)) => a == b,
            (ParamType::Lat(a), ParamType::Lat(b)) => a == b,
            (ParamType::Other(a), ParamType::Other(b)) => a == b,
            _ => false,
        }
    }
}

impl PartialEq for CellParam {
    fn eq(&self, other: &Self) -> bool {
        self.param_type == other.param_type
    }
}

impl TryFrom<&str> for CellParam {
    type Error = ParseError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let param_text = text.as_bytes();
        let keyword = peek_word(param_text, 0).ok_or(ParseError::UnknownParameter)?;
        let mut pos = 0;
        Self::parse(keyword, param_text, &mut pos)
    }
}

impl TryFrom<String> for CellParam {
    type Error = ParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::try_from(text.as_str())
    }
}

/// Parse cell parameters (IMP, VOL, etc.)
pub fn parse_parameters(bytes: &[u8], pos: &mut usize) -> Result<Vec<CellParam>, ParseError> {
    let mut params = Vec::new();

    while *pos < bytes.len() {
        skip_whitespace(bytes, pos);

        if *pos >= bytes.len() {
            break;
        }

        // Parse keyword (including : for things like IMP:N)
        let Some(keyword) = peek_word(bytes, *pos) else {
            break;
        };

        let cell_param = CellParam::parse(keyword, bytes, pos)?;
        params.push(cell_param);
    }

    Ok(params)
}

impl CellParam {
    fn parse(keyword: &[u8], data: &[u8], pos: &mut usize) -> Result<Self, ParseError> {
        let make = |param_type, span| CellParam { param_type, span };
        match keyword.len() {
            1 if keyword.eq_ignore_ascii_case(b"U") => {
                let s = parse_spanned_parameter_u32(data, pos)?;
                Ok(make(ParamType::U(s.value), s.span))
            }
            3 if keyword.eq_ignore_ascii_case(b"VOL") => {
                let s = parse_spanned_parameter_f64(data, pos)?;
                Ok(make(ParamType::Vol(s.value), s.span))
            }
            3 if keyword.eq_ignore_ascii_case(b"TMP") => {
                let s = parse_spanned_parameter_f64(data, pos)?;
                Ok(make(ParamType::Tmp(s.value), s.span))
            }
            3 if keyword.eq_ignore_ascii_case(b"LAT") => {
                let s = parse_spanned_parameter_u32(data, pos)?;
                Ok(make(ParamType::Lat(s.value), s.span))
            }
            4 if keyword.eq_ignore_ascii_case(b"FILL") => {
                let (fill_data, span) = parse_fill_parameters(data, pos)?;
                Ok(make(ParamType::Fill(fill_data), span))
            }
            5 if keyword.eq_ignore_ascii_case(b"*FILL") => {
                let (mut fill_data, span) = parse_fill_parameters(data, pos)?;
                fill_data.starred = true;
                Ok(make(ParamType::Fill(fill_data), span))
            }
            5 if keyword.eq_ignore_ascii_case(b"IMP:N") => {
                let s = parse_spanned_parameter_f64(data, pos)?;
                Ok(make(ParamType::ImpN(s.value), s.span))
            }
            5 if keyword.eq_ignore_ascii_case(b"IMP:P") => {
                let s = parse_spanned_parameter_f64(data, pos)?;
                Ok(make(ParamType::ImpP(s.value), s.span))
            }
            _ => {
                let span = parse_other_parameter_span(data, pos);
                let value = String::from_utf8_lossy(&data[span.0..span.1]).to_string();
                Ok(make(ParamType::Other(value), span))
            }
        }
    }

    pub fn write(&self, source: &[u8], result: &mut Vec<u8>, pos: &mut usize) {
        if self.span.0 == self.span.1 {
            result.push(b' ');
        } else {
            result.extend_from_slice(&source[*pos..self.span.0]);
            *pos = self.span.1;
        }
        self.write_bytes(result);
    }

    pub fn write_bytes(&self, result: &mut Vec<u8>) {
        match &self.param_type {
            ParamType::ImpN(value) => write!(result, "IMP:N={value}").unwrap(),
            ParamType::ImpP(value) => write!(result, "IMP:P={value}").unwrap(),
            ParamType::U(value) => write!(result, "U={value}").unwrap(),
            ParamType::Vol(value) => write!(result, "VOL={}", fmt_mcnp_sci(*value)).unwrap(),
            ParamType::Tmp(value) => write!(result, "TMP={}", fmt_mcnp_sci(*value)).unwrap(),
            ParamType::Lat(value) => write!(result, "LAT={value}").unwrap(),
            ParamType::Other(value) => write!(result, "{}", value).unwrap(),
            ParamType::Fill(FillData {
                starred,
                transform,
                universe,
                coeffs,
            }) => {
                if *starred {
                    write!(result, "*FILL={universe}").unwrap();
                } else {
                    write!(result, "FILL={universe}").unwrap();
                }
                if let Some(t) = transform {
                    write!(result, " ({t})").unwrap();
                } else if let Some(coeffs) = coeffs {
                    result.extend_from_slice(b" (");
                    for (i, c) in coeffs.iter().enumerate() {
                        if i > 0 {
                            result.push(b' ');
                        }
                        write!(result, "{c}").unwrap();
                    }
                    result.push(b')');
                }
            }
        }
    }
}

fn parse_spanned_parameter_u32(data: &[u8], pos: &mut usize) -> Result<Spanned<u32>, ParseError> {
    let start = *pos;
    consume_word(data, pos);
    let value = parse_u32_fast(data, pos).map_err(|_| ParseError::UnknownParameter)?;
    Ok(Spanned::new(value, Span(start, *pos)))
}

fn parse_spanned_parameter_f64(data: &[u8], pos: &mut usize) -> Result<Spanned<f64>, ParseError> {
    let start = *pos;
    consume_word(data, pos);
    let value = parse_f64_fast(data, pos).map_err(|_| ParseError::UnknownParameter)?;
    Ok(Spanned::new(value, Span(start, *pos)))
}

fn is_known_cell_keyword(keyword: &[u8]) -> bool {
    match keyword.len() {
        1 => keyword.eq_ignore_ascii_case(b"U"),
        3 => {
            keyword.eq_ignore_ascii_case(b"VOL")
                || keyword.eq_ignore_ascii_case(b"TMP")
                || keyword.eq_ignore_ascii_case(b"LAT")
        }
        4 => keyword.eq_ignore_ascii_case(b"FILL"),
        5 => {
            keyword.eq_ignore_ascii_case(b"*FILL")
                || keyword.eq_ignore_ascii_case(b"IMP:N")
                || keyword.eq_ignore_ascii_case(b"IMP:P")
        }
        _ => false,
    }
}

fn parse_other_parameter_span(data: &[u8], pos: &mut usize) -> Span {
    let start = *pos;
    consume_word(data, pos);

    loop {
        let end_before_ws = *pos;
        skip_whitespace(data, pos);

        if *pos >= data.len() {
            return Span(start, end_before_ws);
        }

        if let Some(keyword) = peek_word(data, *pos)
            && is_known_cell_keyword(keyword)
        {
            return Span(start, end_before_ws);
        }

        // Consume next token-like chunk or a single non-whitespace byte.
        if consume_word(data, pos).is_none() {
            *pos += 1;
        }
    }
}

fn parse_fill_parameters(bytes: &[u8], pos: &mut usize) -> Result<(FillData, Span), ParseError> {
    skip_whitespace(bytes, pos);
    let start = *pos;
    consume_word(bytes, pos); // Consume "FILL" or "*FILL"

    // Parse universe ID
    let universe = parse_u32_fast(bytes, pos).map_err(|_| ParseError::UnknownParameter)?;

    let pos_before_skip = *pos;
    skip_whitespace(bytes, pos);

    // Check for optional '('
    if bytes.get(*pos) != Some(&b'(') {
        // No parentheses - simple FILL with just universe number
        return Ok((
            FillData {
                starred: false,
                transform: None,
                universe,
                coeffs: None,
            },
            Span(start, pos_before_skip),
        ));
    }
    *pos += 1;

    let mut coeffs = Vec::new();
    loop {
        skip_whitespace(bytes, pos);

        if bytes.get(*pos) == Some(&b')') {
            *pos += 1; // Consume ')'
            break;
        }

        coeffs.push(parse_f64_fast(bytes, pos).map_err(|_| ParseError::UnknownParameter)?);
    }

    if coeffs.len() == 1 {
        let transform = coeffs[0] as u32;
        return Ok((
            FillData {
                starred: false,
                transform: Some(transform),
                universe,
                coeffs: None,
            },
            Span(start, *pos),
        ));
    }

    Ok((
        FillData {
            starred: false,
            transform: None,
            universe,
            coeffs: Some(coeffs),
        },
        Span(start, *pos),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_parameters() {
        let data = b"IMP:N=1.0 VOL = 2.0";
        let mut pos = 0;
        let result = parse_parameters(&data[..], &mut pos);
        let cell_params = result.unwrap();
        assert_eq!(cell_params.len(), 2);
        assert_eq!(cell_params[0].param_type, ParamType::ImpN(1.0));
        assert_eq!(cell_params[1].param_type, ParamType::Vol(2.0));
    }

    #[test]
    fn test_parse_simple_fill() {
        let cell_param = CellParam::try_from("FILL=10040").unwrap();
        assert_eq!(
            cell_param.param_type,
            ParamType::Fill(FillData {
                starred: false,
                transform: None,
                universe: 10040,
                coeffs: None,
            })
        );
    }

    #[test]
    fn test_parse_fill_with_transform() {
        let cell_param = CellParam::try_from("FILL = 5 (130)").unwrap();
        assert_eq!(
            cell_param.param_type,
            ParamType::Fill(FillData {
                starred: false,
                transform: Some(130),
                universe: 5,
                coeffs: None,
            })
        );
    }

    #[test]
    fn test_parse_starred_fill() {
        let cell_param =
            CellParam::try_from("*FILL = 5 (0.001 0.001 0.001 130 40 90 140 130 90 90 90 0 )")
                .unwrap();
        assert_eq!(
            cell_param.param_type,
            ParamType::Fill(FillData {
                starred: true,
                transform: None,
                universe: 5,
                coeffs: Some(vec![
                    0.001, 0.001, 0.001, 130.0, 40.0, 90.0, 140.0, 130.0, 90.0, 90.0, 90.0, 0.0
                ]),
            })
        );
    }

    #[test]
    fn test_parse_other_until_next_keyword() {
        let data = b"UNKNOWN=7 8 VOL=2.0";
        let mut pos = 0;
        let params = parse_parameters(data, &mut pos).unwrap();

        assert_eq!(params.len(), 2);
        assert_eq!(
            params[0].param_type,
            ParamType::Other("UNKNOWN=7 8".to_string())
        ); // We don't check the content of the string here, just that it was parsed as "Other"
        assert_eq!(&data[params[0].span.0..params[0].span.1], b"UNKNOWN=7 8");
        assert_eq!(params[1].param_type, ParamType::Vol(2.0));
    }

    #[test]
    fn test_write_other_preserves_original_bytes() {
        let source = b"UNKNOWN=1 2 U=3";
        let mut pos = 0;
        let params = parse_parameters(source, &mut pos).unwrap();

        let mut out = Vec::new();
        let mut write_pos = 0;
        for p in &params {
            p.write(source, &mut out, &mut write_pos);
        }
        out.extend_from_slice(&source[write_pos..]);

        assert_eq!(out, source);
    }
}
