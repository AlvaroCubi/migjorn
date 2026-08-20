//! Recoverability reporting.
//!
//! The parser never fails. Anything it cannot make sense of becomes a
//! `Diagnostic`, the bytes are preserved exactly, and the model still
//! round-trips.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Byte range in the source **as parsed**. Edits do not move these; they
    /// describe the last parse.
    pub span: Range<usize>,
}

impl Diagnostic {
    pub(crate) fn error(message: String, span: Range<usize>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message,
            span,
        }
    }

    pub(crate) fn warning(message: String, span: Range<usize>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            message,
            span,
        }
    }
}

/// A diagnostic recorded against a card, before card offsets are known.
pub(crate) struct Pending {
    pub card: usize,
    pub local: Range<usize>,
    pub severity: Severity,
    pub message: String,
}
