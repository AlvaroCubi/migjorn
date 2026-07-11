//! `crunchy-core` — the typed AST layer over the lossless `crunchy-syntax` CST.
//!
//! Typed views (`Surface`, `Transform`, …) are projected on demand from the
//! flat CST. They carry the token indices they were built from, so higher-level
//! edits (e.g. renumbering) can rewrite the exact tokens while the CST keeps the
//! rest of the deck byte-for-byte intact.

mod cell;
mod num;
mod surface;
mod transform;

pub use cell::{cells, parse_cell, Cell, CellRef, GeomExpr, SurfaceRef};
pub use num::{parse_float, parse_int};
pub use surface::{parse_surface, surfaces, Surface, SurfaceKind};
pub use transform::{parse_transform, transforms, Transform};

// Re-export the syntax layer so downstream users need only depend on core.
pub use crunchy_syntax::{self as syntax, parse, Card, Diagnostic, GreenTree, Parsed, Severity, Span, SyntaxKind};
