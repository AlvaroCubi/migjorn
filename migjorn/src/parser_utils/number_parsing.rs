use super::character_class::{CHAR_CLASS, DIGIT};
use super::parser_errors::PrimitiveError;
use super::skip_whitespace;

/// Fast integer parsing (handles negative)
#[inline]
pub fn parse_i32_fast(bytes: &[u8], pos: &mut usize) -> Result<i32, PrimitiveError> {
    skip_whitespace(bytes, pos);

    if *pos >= bytes.len() {
        return Err(PrimitiveError::UnexpectedEnd);
    }

    let mut negative = false;
    if bytes[*pos] == b'-' {
        negative = true;
        *pos += 1;
    } else if bytes[*pos] == b'+' {
        *pos += 1;
    }

    let mut value: i32 = 0;
    let mut found_digit = false;

    while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & DIGIT != 0 {
        found_digit = true;
        let digit = (bytes[*pos] - b'0') as i32;
        value = value.wrapping_mul(10).wrapping_add(digit);
        *pos += 1;
    }

    if !found_digit {
        return Err(PrimitiveError::InvalidInteger);
    }

    Ok(if negative { -value } else { value })
}

#[inline]
pub fn parse_u32_fast(bytes: &[u8], pos: &mut usize) -> Result<u32, PrimitiveError> {
    skip_whitespace(bytes, pos);

    if *pos >= bytes.len() {
        return Err(PrimitiveError::UnexpectedEnd);
    }

    if bytes[*pos] == b'-' {
        return Err(PrimitiveError::InvalidUnsigned);
    } else if bytes[*pos] == b'+' {
        *pos += 1;
    }

    let mut value: u32 = 0;
    let mut found_digit = false;

    while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & DIGIT != 0 {
        found_digit = true;
        let digit = (bytes[*pos] - b'0') as u32;
        value = value.wrapping_mul(10).wrapping_add(digit);
        *pos += 1;
    }

    if !found_digit {
        return Err(PrimitiveError::InvalidUnsigned);
    }

    Ok(value)
}

/// Fast float parsing (handles scientific notation)
#[inline]
pub fn parse_f64_fast(bytes: &[u8], pos: &mut usize) -> Result<f64, PrimitiveError> {
    skip_whitespace(bytes, pos);

    if *pos >= bytes.len() {
        return Err(PrimitiveError::UnexpectedEnd);
    }

    let start = *pos;

    // Skip sign
    if bytes[*pos] == b'-' || bytes[*pos] == b'+' {
        *pos += 1;
    }

    // Parse digits before decimal
    while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & DIGIT != 0 {
        *pos += 1;
    }

    // Parse decimal part
    if *pos < bytes.len() && bytes[*pos] == b'.' {
        *pos += 1;
        while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & DIGIT != 0 {
            *pos += 1;
        }
    }

    // Parse exponent (e or E)
    if *pos < bytes.len() && (bytes[*pos] == b'e' || bytes[*pos] == b'E') {
        *pos += 1;
        if *pos < bytes.len() && (bytes[*pos] == b'-' || bytes[*pos] == b'+') {
            *pos += 1;
        }
        while *pos < bytes.len() && CHAR_CLASS[bytes[*pos] as usize] & DIGIT != 0 {
            *pos += 1;
        }
    }

    // Convert to string and parse (minimal allocation)
    let float_str =
        std::str::from_utf8(&bytes[start..*pos]).map_err(|_| PrimitiveError::InvalidFloat)?;

    float_str
        .parse::<f64>()
        .map_err(|_| PrimitiveError::InvalidFloat)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_fast_integer_parsing() {
        let bytes = b"12345 ";
        let mut pos = 0;
        let result = parse_i32_fast(bytes, &mut pos).unwrap();
        assert_eq!(result, 12345);

        let bytes = b"-999";
        let mut pos = 0;
        let result = parse_i32_fast(bytes, &mut pos).unwrap();
        assert_eq!(result, -999);
    }

    #[test]
    fn test_fast_float_parsing() {
        let bytes = b"3.14159";
        let mut pos = 0;
        let result = parse_f64_fast(bytes, &mut pos).unwrap();
        assert!((result - 3.14159).abs() < 1e-10);

        let bytes = b"-2.7e-3";
        let mut pos = 0;
        let result = parse_f64_fast(bytes, &mut pos).unwrap();
        assert!((result - (-2.7e-3)).abs() < 1e-10);
    }
}
