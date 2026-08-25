//! Lexical and structural categories.

/// Lexical categories.
///
/// Unlike the previous engine's tokens, these do **not** tile the card's text:
/// whitespace and newlines produce no token at all. Losslessness comes from the
/// card owning its exact bytes ([`crate::Card::text`]), so tokens are purely an
/// *index* into that text. Comments and `&` are kept as tokens because parsing
/// needs to skip them explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SyntaxKind {
    /// A numeric word: `1`, `-1.0`, `1e-3`, `+558.36`, a ZAID like `1001.31c`,
    /// or a data-array shortcut like `3R` / `8I` / `2J` / `3M` (kept verbatim,
    /// never expanded).
    Number,
    /// A word starting with a letter: mnemonics, keywords, `LIKE`, `BUT`,
    /// particle designators, `lwtr.10t`.
    Ident,
    Colon,
    Eq,
    Star,
    Hash,
    LParen,
    RParen,
    /// A `$ ...` inline comment, or a whole `c ...` comment line absorbed into a
    /// card — either sandwiched before a continuation or heading the card as a
    /// leading comment run. Excludes the line terminator.
    Comment,
    /// A `&` continuation marker.
    Ampersand,
    /// Any word we cannot classify. Recoverability: never a panic, and the bytes
    /// are preserved like everything else.
    Unknown,
}

impl SyntaxKind {
    /// Trivia is preserved but skipped by every typed projection.
    #[inline]
    pub fn is_trivia(self) -> bool {
        matches!(self, SyntaxKind::Comment | SyntaxKind::Ampersand)
    }
}

/// What kind of card this is, decided at segmentation time from block position
/// and the leading bytes.
///
/// `Comment` and `Blank` are structural trivia cards: they carry text but no
/// typed meaning. Keeping them as first-class cards is what makes insert/remove
/// preserve layout exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CardKind {
    /// The single free-text line that names the model. Positional: it is
    /// whatever line follows the optional MESSAGE block, even if it looks like a
    /// comment (the real 380 MB model's title is the single byte `C`).
    Title,
    Cell,
    Surface,
    Data,
    /// A standalone full-line comment: one that is not attached to any card,
    /// i.e. immediately followed by a blank line or the end of a block. (A
    /// comment line sandwiched between a card line and its continuation, or
    /// heading a card with no blank line in between, is *absorbed* into that
    /// card instead.)
    Comment,
    /// A blank line. Load-bearing: blank lines delimit the three blocks.
    Blank,
    /// A line of the optional leading `MESSAGE:` block.
    Message,
    /// Anything after the data block's terminating blank line, preserved verbatim.
    Trailing,
}

impl CardKind {
    /// Cards that carry typed content (as opposed to layout trivia).
    #[inline]
    pub fn is_content(self) -> bool {
        matches!(self, CardKind::Cell | CardKind::Surface | CardKind::Data)
    }
}

/// The line terminator a file uses. Detected at parse and reused for cards the
/// user adds later, so an edit to a CRLF file does not introduce a lone `\n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
}

impl Eol {
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::Crlf => "\r\n",
        }
    }

    /// Detect from the first line terminator in `src`; defaults to `\n`.
    pub fn detect(src: &str) -> Eol {
        match memchr::memchr(b'\n', src.as_bytes()) {
            Some(i) if i > 0 && src.as_bytes()[i - 1] == b'\r' => Eol::Crlf,
            _ => Eol::Lf,
        }
    }
}
