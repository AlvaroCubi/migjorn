//! Numeric parsing for MCNP.
//!
//! MCNP accepts a Fortran-ish float syntax that Rust's `f64::from_str` rejects:
//! an exponent may omit the `e`, e.g. `1.0-5` means `1.0e-5` and `6.02+23`
//! means `6.02e23`. We first try the standard parse, then fall back to
//! reconstructing the missing `e`.

/// Parse an MCNP numeric literal into `f64`, accepting exponent-without-`e`
/// forms (`1.0-5`, `6.02+23`). Returns `None` if the token is not numeric.
pub fn parse_float(s: &str) -> Option<f64> {
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    // Look for a sign that introduces an implicit exponent: a `+`/`-` that is
    // not the leading character and is not already preceded by `e`/`E`.
    let bytes = s.as_bytes();
    for i in 1..bytes.len() {
        let b = bytes[i];
        if b == b'+' || b == b'-' {
            let prev = bytes[i - 1];
            if prev != b'e' && prev != b'E' {
                let mut fixed = String::with_capacity(s.len() + 1);
                fixed.push_str(&s[..i]);
                fixed.push('e');
                fixed.push_str(&s[i..]);
                if let Ok(v) = fixed.parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Parse an MCNP integer literal (allows a leading `+`).
pub fn parse_int(s: &str) -> Option<i64> {
    let s = s.strip_prefix('+').unwrap_or(s);
    s.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_floats() {
        assert_eq!(parse_float("3.5"), Some(3.5));
        assert_eq!(parse_float("-1.0e-3"), Some(-1.0e-3));
        assert_eq!(parse_float(".5"), Some(0.5));
        assert_eq!(parse_float("+2"), Some(2.0));
    }

    #[test]
    fn mcnp_implicit_exponent() {
        assert_eq!(parse_float("1.0-5"), Some(1.0e-5));
        assert_eq!(parse_float("6.02+23"), Some(6.02e23));
        assert_eq!(parse_float("-1.0-5"), Some(-1.0e-5));
        assert_eq!(parse_float("2.0+0"), Some(2.0));
    }

    #[test]
    fn non_numeric() {
        assert_eq!(parse_float("PX"), None);
        assert_eq!(parse_float("1.0-"), None);
    }

    #[test]
    fn integers() {
        assert_eq!(parse_int("113"), Some(113));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("-3"), Some(-3));
    }
}
