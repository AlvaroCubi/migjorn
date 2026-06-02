use std::fmt::Display;
use std::sync::Arc;

/// Storage for a card's raw bytes.
///
/// - [`OriginalBytes::Source`]: a zero-copy slice into the shared, immutable file
///   buffer.  This is the common case for cards parsed from a file.  Dropping
///   a model with a million `Source` cards costs a single `Arc` decrement —
///   no per-card heap-frees.
/// - [`OriginalBytes::Owned`]: heap-allocated bytes for new or modified cards.
#[derive(Debug, Clone)]
pub enum OriginalBytes {
    Source {
        source: Arc<[u8]>,
        offset: usize,
        len: usize,
    },
    Owned(Box<[u8]>),
}

impl AsRef<[u8]> for OriginalBytes {
    /// Returns the raw bytes of this card as a slice.
    fn as_ref(&self) -> &[u8] {
        match self {
            OriginalBytes::Source {
                source,
                offset,
                len,
            } => &source[*offset..*offset + *len],
            OriginalBytes::Owned(bytes) => bytes,
        }
    }
}

impl Display for OriginalBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(self.as_ref()))
    }
}

impl OriginalBytes {
    /// Returns the raw bytes of this card as a UTF-8 string slice.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_ref()).expect("Card bytes should be valid UTF-8")
    }

    pub fn len(&self) -> usize {
        match self {
            OriginalBytes::Source { len, .. } => *len,
            OriginalBytes::Owned(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the byte offset of this card inside the original source buffer,
    /// or `None` for owned (non-source) cards.
    pub fn source_offset(&self) -> Option<usize> {
        match self {
            OriginalBytes::Source { offset, .. } => Some(*offset),
            OriginalBytes::Owned(_) => None,
        }
    }
}

impl From<Box<[u8]>> for OriginalBytes {
    fn from(bytes: Box<[u8]>) -> Self {
        OriginalBytes::Owned(bytes)
    }
}

impl From<Vec<u8>> for OriginalBytes {
    fn from(v: Vec<u8>) -> Self {
        OriginalBytes::Owned(v.into_boxed_slice())
    }
}

impl From<&[u8]> for OriginalBytes {
    fn from(bytes: &[u8]) -> Self {
        OriginalBytes::Owned(Box::from(bytes))
    }
}

impl From<&str> for OriginalBytes {
    fn from(s: &str) -> Self {
        OriginalBytes::Owned(Box::from(s.as_bytes()))
    }
}

/// Two `OriginalBytes` values are equal if their byte content is identical,
/// regardless of whether they are `Source` or `Owned`.
impl PartialEq for OriginalBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
