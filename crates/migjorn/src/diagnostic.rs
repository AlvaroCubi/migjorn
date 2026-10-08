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
    ///
    /// Diagnostics read off a single card (`CellView::diagnostics`, ...) are
    /// the exception: their span is relative to that card's own current text.
    pub span: Range<usize>,
    /// The card the diagnostic is about, as a stable slot (resolve it with
    /// `Model::cell_at`, `Model::surface_at`, ...). `None` only for problems
    /// that belong to no single card.
    pub slot: Option<u32>,
}

impl Diagnostic {
    pub(crate) fn error(message: String, span: Range<usize>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message,
            span,
            slot: None,
        }
    }

    pub(crate) fn warning(message: String, span: Range<usize>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            message,
            span,
            slot: None,
        }
    }

    pub(crate) fn on_slot(mut self, slot: u32) -> Diagnostic {
        self.slot = Some(slot);
        self
    }

    /// The 1-based line number containing `span.start`, counted in `source` —
    /// the same source text the diagnostic was produced against. Clamps an
    /// out-of-range offset to the end of `source` rather than panicking, so a
    /// stale `source` (edited since the diagnostic was recorded) can't panic
    /// a caller that only wants an approximate location.
    pub fn line(&self, source: &str) -> usize {
        let offset = self.span.start.min(source.len());
        source.as_bytes()[..offset]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
            + 1
    }
}

/// A diagnostic recorded against a card, before card offsets are known.
pub(crate) struct Pending {
    pub card: usize,
    pub slot: u32,
    pub local: Range<usize>,
    pub severity: Severity,
    pub message: String,
}

impl Pending {
    /// As a `Diagnostic` whose span starts at byte `base` of the source.
    pub(crate) fn at(self, base: usize) -> Diagnostic {
        let span = base + self.local.start..base + self.local.end;
        match self.severity {
            Severity::Error => Diagnostic::error(self.message, span),
            Severity::Warning => Diagnostic::warning(self.message, span),
        }
        .on_slot(self.slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_counts_newlines_before_the_span_start() {
        let source = "first\nsecond\nthird";
        let at = |start: usize| Diagnostic::error("x".to_string(), start..start).line(source);
        assert_eq!(at(0), 1);
        assert_eq!(at(6), 2); // start of "second"
        assert_eq!(at(13), 3); // start of "third"
        assert_eq!(at(source.len()), 3);
    }

    #[test]
    fn line_clamps_a_span_start_past_the_end_of_source() {
        let diag = Diagnostic::error("x".to_string(), 100..100);
        assert_eq!(diag.line("abc"), 1);
    }
}
