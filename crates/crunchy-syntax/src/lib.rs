//! `crunchy-syntax` — the lossless syntax layer for the crunchy MCNP parser.
//!
//! This crate owns the lexer and the concrete syntax tree (CST). Higher layers
//! (`crunchy-core`) project a typed AST on top of it.

mod cst;
mod diagnostics;
mod lexer;
mod syntax_kind;

pub use cst::{parse, Card, GreenTree, Parsed};
pub use diagnostics::{Diagnostic, Severity, Span};
pub use lexer::lex;
pub use syntax_kind::SyntaxKind;
