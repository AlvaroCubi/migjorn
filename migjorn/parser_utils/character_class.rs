// Character classification lookup table (compile-time constant)
pub const WHITESPACE: u8 = 1;
pub const DIGIT: u8 = 2;
pub const SIGN: u8 = 4;
pub const GEOMETRY_OP: u8 = 8;
pub const KEYWORD_CHAR: u8 = 16;
pub const LINE_COMMENT: u8 = 32;

pub static CHAR_CLASS: [u8; 256] = {
    let mut table = [0u8; 256];

    // Whitespace
    table[b' ' as usize] = WHITESPACE;
    table[b'\t' as usize] = WHITESPACE;
    table[b'\n' as usize] = WHITESPACE;
    table[b'\r' as usize] = WHITESPACE;
    table[b'=' as usize] = WHITESPACE;

    // Digits
    table[b'0' as usize] = DIGIT;
    table[b'1' as usize] = DIGIT;
    table[b'2' as usize] = DIGIT;
    table[b'3' as usize] = DIGIT;
    table[b'4' as usize] = DIGIT;
    table[b'5' as usize] = DIGIT;
    table[b'6' as usize] = DIGIT;
    table[b'7' as usize] = DIGIT;
    table[b'8' as usize] = DIGIT;
    table[b'9' as usize] = DIGIT;

    // Signs
    table[b'+' as usize] = SIGN;
    table[b'-' as usize] = SIGN;

    // Geometry operators
    table[b'(' as usize] = GEOMETRY_OP;
    table[b')' as usize] = GEOMETRY_OP;
    table[b':' as usize] = GEOMETRY_OP;
    table[b'#' as usize] = GEOMETRY_OP;

    // Letters (for keywords)
    let mut i = b'A';
    while i <= b'Z' {
        table[i as usize] = KEYWORD_CHAR;
        i += 1;
    }
    let mut i = b'a';
    while i <= b'z' {
        table[i as usize] = KEYWORD_CHAR;
        i += 1;
    }
    table[b'c' as usize] |= LINE_COMMENT;
    table[b'C' as usize] |= LINE_COMMENT;

    table[b'/' as usize] = KEYWORD_CHAR; // Allow '/' in keywords (e.g. C/Z)
    table[b'*' as usize] = KEYWORD_CHAR; // Allow '*' in keywords (e.g. *F4)
    table[b':' as usize] |= KEYWORD_CHAR; // Allow ':' in keywords (e.g. IMP:N)

    table[b',' as usize] = KEYWORD_CHAR; // Allow ',' in keywords (e.g. IMP:N,P)

    table
};
