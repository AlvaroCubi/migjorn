//! The flat set of token/node kinds for the MCNP concrete syntax tree.
//!
//! `SyntaxKind` is `#[repr(u16)]` with contiguous discriminants so it can be
//! transmuted to/from `rowan::SyntaxKind` cheaply. Keep `__LAST` at the end.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum SyntaxKind {
    // ---- tokens (leaves) ----
    /// UTF-8 byte-order mark at the very start of the file.
    BOM = 0,
    /// A run of spaces and/or tabs (never contains a newline).
    WHITESPACE,
    /// A line terminator: `\n`, `\r`, or `\r\n`.
    NEWLINE,
    /// A full-line comment (`c`/`C` in the name field through end of line).
    COMMENT_LINE,
    /// An inline comment introduced by `$` through end of line.
    DOLLAR_COMMENT,
    /// A numeric literal (integer or float, including MCNP `1.0-5` exponents).
    NUMBER,
    /// An identifier / mnemonic / keyword (`PX`, `LIKE`, `imp`, ...).
    IDENT,
    /// `:` — geometry union or particle-designator separator.
    COLON,
    /// `#` — cell complement.
    HASH,
    /// `(`
    L_PAREN,
    /// `)`
    R_PAREN,
    /// `&` — explicit line continuation.
    AMP,
    /// `=` — keyword-parameter separator (`FILE=...`).
    EQ,
    /// `*` — modifier prefix (`*TR`, `*F`, reflective surface).
    STAR,
    /// Any byte we do not otherwise classify.
    UNKNOWN,

    // ---- nodes (interior) ----
    /// A message block card (precedes the title, optional).
    MESSAGE_CARD,
    /// The title card (first logical line of the deck).
    TITLE_CARD,
    /// A cell card (a logical line in the cell block).
    CELL_CARD,
    /// A surface card (a logical line in the surface block).
    SURFACE_CARD,
    /// A data card (a logical line in the data block).
    DATA_CARD,
    /// One physical source line (grouping used by the M0 spike).
    LINE,
    /// The whole file.
    ROOT,

    #[doc(hidden)]
    __LAST,
}

impl SyntaxKind {
    /// Reconstruct a `SyntaxKind` from its `u16` discriminant.
    #[inline]
    pub fn from_u16(v: u16) -> SyntaxKind {
        assert!(v < SyntaxKind::__LAST as u16, "invalid SyntaxKind {v}");
        // SAFETY: contiguous `#[repr(u16)]` discriminants in `0..__LAST`.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(v) }
    }

    /// True for tokens that carry no semantic content (whitespace, comments,
    /// BOM). The typed layer skips these when reading a card.
    #[inline]
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE
                | SyntaxKind::NEWLINE
                | SyntaxKind::COMMENT_LINE
                | SyntaxKind::DOLLAR_COMMENT
                | SyntaxKind::BOM
        )
    }

    /// True for the card-level node kinds.
    #[inline]
    pub fn is_card(self) -> bool {
        matches!(
            self,
            SyntaxKind::MESSAGE_CARD
                | SyntaxKind::TITLE_CARD
                | SyntaxKind::CELL_CARD
                | SyntaxKind::SURFACE_CARD
                | SyntaxKind::DATA_CARD
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    #[inline]
    fn from(k: SyntaxKind) -> Self {
        rowan::SyntaxKind(k as u16)
    }
}

/// The rowan `Language` binding for MCNP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum McnpLang {}

impl rowan::Language for McnpLang {
    type Kind = SyntaxKind;

    #[inline]
    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        assert!(raw.0 < SyntaxKind::__LAST as u16, "invalid SyntaxKind {}", raw.0);
        // SAFETY: `SyntaxKind` is `#[repr(u16)]` with contiguous discriminants
        // in `0..__LAST`, and we just checked the bound.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    #[inline]
    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<McnpLang>;
pub type SyntaxToken = rowan::SyntaxToken<McnpLang>;
pub type SyntaxElement = rowan::SyntaxElement<McnpLang>;
