use std::fmt::Display;

use crate::Card;
use crate::parser_utils::{
    CHAR_CLASS, CardError, DIGIT, OriginalBytes, ParseError, Span, Spanned, approx_eq_f64,
    consume_word, parse_f64_fast, parse_spanned_u32, skip_whitespace, write_spanned,
};

#[derive(Debug, Clone)]
pub struct SurfaceCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) surface_id: Spanned<u32>,
    pub(crate) transform_id: Option<Spanned<u32>>,
    pub(crate) surface_type: SurfaceType,
    pub(crate) coeffs: Vec<f64>,
}

impl PartialEq for SurfaceCard {
    fn eq(&self, other: &Self) -> bool {
        self.surface_id.value == other.surface_id.value
            && self.transform_id.as_ref().map(|t| t.value)
                == other.transform_id.as_ref().map(|t| t.value)
            && self.surface_type == other.surface_type
            && self.coeffs.len() == other.coeffs.len()
            && self
                .coeffs
                .iter()
                .zip(other.coeffs.iter())
                .all(|(&a, &b)| approx_eq_f64(a, b))
    }
}

impl SurfaceCard {
    pub fn surface_id(&self) -> u32 {
        self.surface_id.value
    }

    pub fn set_surface_id(&mut self, new_id: u32) {
        self.surface_id.value = new_id;
    }

    pub fn transform_id(&self) -> Option<u32> {
        self.transform_id.as_ref().map(|t| t.value)
    }

    pub fn set_transform_id(&mut self, new_id: u32) {
        if let Some(transform_id) = &mut self.transform_id {
            transform_id.value = new_id;
        } else {
            // No existing transform ID: insert one as a zero-length span immediately
            // after the surface ID. write_spanned will prepend a space automatically.
            self.transform_id = Some(Spanned::new(new_id, Span::empty_at(self.surface_id.span.1)));
        }
    }

    /// No setter as they are not spanned, written as they appear in the original bytes
    pub fn surface_type(&self) -> &SurfaceType {
        &self.surface_type
    }

    /// No setter as they are not spanned, written as they appear in the original bytes
    pub fn coeffs(&self) -> &[f64] {
        &self.coeffs
    }
}

impl Card for SurfaceCard {
    fn original_bytes(&self) -> &[u8] {
        self.original_bytes.as_ref()
    }

    fn updated_bytes(&self) -> Vec<u8> {
        let src = self.original_bytes.as_ref();
        let mut result = Vec::with_capacity(src.len());
        let mut pos = 0;
        write_spanned(src, &mut result, &mut pos, &self.surface_id);
        if let Some(transform_id) = &self.transform_id {
            write_spanned(src, &mut result, &mut pos, transform_id);
        }
        result.extend_from_slice(&src[pos..]);
        result
    }
}

impl TryFrom<OriginalBytes> for SurfaceCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        SurfaceCard::parse(bytes)
    }
}

impl TryFrom<&str> for SurfaceCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        SurfaceCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for SurfaceCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SurfaceCard
    surface_id: {},
    transform_id: {:?},
    surface_type: {:?},
    coeffs: {:?}",
            self.surface_id.value, self.transform_id, self.surface_type, self.coeffs,
        )
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum SurfaceType {
    P,
    PX,
    PY,
    PZ,
    CX,
    CY,
    CZ,
    CXp,
    CYp,
    CZp,
    TX,
    TY,
    TZ,
    S,
    SO,
    SX,
    SY,
    SZ,
    SQ,
    KX,
    KY,
    KZ,
    KXp,
    KYp,
    KZp,
    GQ,
    Box,
    Rpp,
    Sph,
    Rcc,
    Rhp,
    Rec,
    Trc,
    Ell,
    Wed,
    Arb,
    X,
    Y,
    Z,
}

#[rustfmt::skip]
impl SurfaceType {
    pub fn parse(keyword: &[u8]) -> Result<Self, ParseError> {
        let eq = |expected: &[u8]| keyword.eq_ignore_ascii_case(expected);

        match keyword.len() {
            1 => {
                if eq(b"P") { return Ok(Self::P); }
                if eq(b"S") { return Ok(Self::S); }
                if eq(b"X") { return Ok(Self::X); }
                if eq(b"Y") { return Ok(Self::Y); }
                if eq(b"Z") { return Ok(Self::Z); }
            }
            2 => {
                if eq(b"PX") { return Ok(Self::PX); }
                if eq(b"PY") { return Ok(Self::PY); }
                if eq(b"PZ") { return Ok(Self::PZ); }
                if eq(b"CX") { return Ok(Self::CX); }
                if eq(b"CY") { return Ok(Self::CY); }
                if eq(b"CZ") { return Ok(Self::CZ); }
                if eq(b"TX") { return Ok(Self::TX); }
                if eq(b"TY") { return Ok(Self::TY); }
                if eq(b"TZ") { return Ok(Self::TZ); }
                if eq(b"SO") { return Ok(Self::SO); }
                if eq(b"SX") { return Ok(Self::SX); }
                if eq(b"SY") { return Ok(Self::SY); }
                if eq(b"SZ") { return Ok(Self::SZ); }
                if eq(b"SQ") { return Ok(Self::SQ); }
                if eq(b"KX") { return Ok(Self::KX); }
                if eq(b"KY") { return Ok(Self::KY); }
                if eq(b"KZ") { return Ok(Self::KZ); }
                if eq(b"GQ") { return Ok(Self::GQ); }
            }
            3 => {
                if eq(b"C/X") { return Ok(Self::CXp); }
                if eq(b"C/Y") { return Ok(Self::CYp); }
                if eq(b"C/Z") { return Ok(Self::CZp); }
                if eq(b"K/X") { return Ok(Self::KXp); }
                if eq(b"K/Y") { return Ok(Self::KYp); }
                if eq(b"K/Z") { return Ok(Self::KZp); }
                if eq(b"BOX") { return Ok(Self::Box); }
                if eq(b"RPP") { return Ok(Self::Rpp); }
                if eq(b"SPH") { return Ok(Self::Sph); }
                if eq(b"RCC") { return Ok(Self::Rcc); }
                if eq(b"RHP") { return Ok(Self::Rhp); }
                if eq(b"REC") { return Ok(Self::Rec); }
                if eq(b"TRC") { return Ok(Self::Trc); }
                if eq(b"ELL") { return Ok(Self::Ell); }
                if eq(b"WED") { return Ok(Self::Wed); }
                if eq(b"ARB") { return Ok(Self::Arb); }
            }
            _ => {}
        }

        Err(ParseError::InvalidSurfType)
    }
}

impl SurfaceCard {
    fn parse(original_bytes: OriginalBytes) -> Result<SurfaceCard, CardError> {
        let bytes = original_bytes.as_ref();
        let mut pos = 0;

        // Skip initial whitespace and comments
        skip_whitespace(bytes, &mut pos);

        if pos >= bytes.len() {
            return Err(CardError::new(ParseError::EmptyCard, original_bytes, pos));
        }

        // If the card starts with '*' or '+' (reflective surface), skip the character
        // we dont store reflective information for now
        if bytes[pos] == b'*' || bytes[pos] == b'+' {
            pos += 1;
        }

        // Parse surface ID
        let surface_id = parse_spanned_u32(bytes, &mut pos).map_err(|_| {
            CardError::new(ParseError::InvalidSurfaceId, original_bytes.clone(), pos)
        })?;

        // Try to parse optional transformation ID
        skip_whitespace(bytes, &mut pos);
        if pos >= bytes.len() {
            return Err(CardError::new(
                ParseError::MissingSurfType,
                original_bytes.clone(),
                pos,
            ));
        }

        // Check if the next token is a number (transformation) or letters (surface type)
        let transform_id = (CHAR_CLASS[bytes[pos] as usize] & DIGIT != 0)
            .then(|| {
                parse_spanned_u32(bytes, &mut pos).map_err(|_| {
                    CardError::new(ParseError::InvalidTransformId, original_bytes.clone(), pos)
                })
            })
            .transpose()?;

        // Parse surface type
        let keyword = consume_word(bytes, &mut pos).ok_or_else(|| {
            CardError::new(ParseError::MissingSurfType, original_bytes.clone(), pos)
        })?;
        let surface_type = SurfaceType::parse(keyword).map_err(|_| {
            CardError::new(ParseError::InvalidSurfType, original_bytes.clone(), pos)
        })?;

        // Parse coefficients
        let mut coeffs = Vec::new();
        while pos < bytes.len() {
            skip_whitespace(bytes, &mut pos);
            if pos >= bytes.len() {
                break;
            }
            let coeff = parse_f64_fast(bytes, &mut pos).map_err(|_| {
                CardError::new(ParseError::InvalidCoefficient, original_bytes.clone(), pos)
            })?;
            coeffs.push(coeff);
        }

        Ok(SurfaceCard {
            original_bytes,
            surface_id,
            transform_id,
            surface_type,
            coeffs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_surface_card() {
        let surface_card = SurfaceCard::try_from("1001 pz 1.2e3").unwrap();
        assert_eq!(surface_card.surface_id.value, 1001);
        assert!(surface_card.transform_id.is_none());
        assert_eq!(surface_card.surface_type, SurfaceType::PZ);
        assert_eq!(surface_card.coeffs, vec![1.2e3]);
    }

    #[test]
    fn parse_complex_surface_card() {
        let surface_card =
            SurfaceCard::try_from("*2002 5 CX 1.0 2.0 3.0 4.0e-2 5.0 $ comment").unwrap();
        assert_eq!(surface_card.surface_id.value, 2002);
        assert_eq!(surface_card.transform_id.map(|t| t.value), Some(5));
        assert_eq!(surface_card.surface_type, SurfaceType::CX);
        assert_eq!(surface_card.coeffs, vec![1.0, 2.0, 3.0, 4.0e-2, 5.0]);

        // Test that it also works with '+' surfaces
        let surface_card =
            SurfaceCard::try_from("+2002 5 CX 1.0 2.0 3.0 4.0e-2 5.0 $ comment").unwrap();
        assert_eq!(surface_card.surface_id.value, 2002);
    }

    #[test]
    fn get_and_set_surface_id() {
        let mut surface_card = SurfaceCard::try_from("3003 PX 0.5").unwrap();
        assert_eq!(surface_card.surface_id(), 3003);
        surface_card.set_surface_id(4004);
        assert_eq!(surface_card.surface_id(), 4004);
        assert_eq!(surface_card.updated_text(), "4004 PX 0.5");
    }

    #[test]
    fn get_and_set_transform_id() {
        let mut surface_card = SurfaceCard::try_from("5005 PY 2.5").unwrap();
        assert!(surface_card.transform_id().is_none());
        surface_card.set_transform_id(10);
        assert_eq!(surface_card.transform_id(), Some(10));
        assert_eq!(surface_card.updated_text(), "5005 10 PY 2.5");
        surface_card.set_transform_id(20);
        assert_eq!(surface_card.transform_id(), Some(20));
        assert_eq!(surface_card.updated_text(), "5005 20 PY 2.5");
    }
}
