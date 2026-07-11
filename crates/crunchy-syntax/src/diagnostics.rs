//! Diagnostics produced during parsing.
//!
//! The parser is *recoverable*: it never aborts on malformed input. Instead it
//! records a `Diagnostic` (with a byte span) and continues, so a single parse
//! can surface many problems at once.

/// Byte range `[start, end)` into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single diagnostic message anchored to a source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            span,
            severity: Severity::Error,
            message: message.into(),
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            span,
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}
