//! `crunchy-core` — the typed AST layer over the lossless `crunchy-syntax` CST.
//!
//! Typed views (`Surface`, `Transform`, …) are projected on demand from the
//! flat CST. They carry the token indices they were built from, so higher-level
//! edits (e.g. renumbering) can rewrite the exact tokens while the CST keeps the
//! rest of the model byte-for-byte intact.

mod cell;
mod datacard;
mod material;
mod model;
mod num;
mod renumber;
mod surface;
mod transform;

pub use cell::{
    cell_id, cells, parse_cell, scan_cell_refs, Cell, CellRef, GeomExpr, RefKind, SurfaceRef,
};
pub use datacard::{data_cards, parse_data_card, DataCard};
pub use material::{materials, parse_material, Material, MaterialEntry};
pub use model::{Model, ModelIndex};
pub use num::{parse_float, parse_int};
pub use renumber::{renumber_cells, renumber_surfaces};
pub use surface::{parse_surface, surface_id, surfaces, Surface, SurfaceKind};
pub use transform::{parse_transform, transforms, Transform};

// Re-export the syntax layer so downstream users need only depend on core.
pub use crunchy_syntax::{
    self as syntax, parse, Card, Diagnostic, GreenTree, Parsed, Severity, Span, SyntaxKind,
};
