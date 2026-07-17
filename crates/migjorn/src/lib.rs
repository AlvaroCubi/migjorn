//! `migjorn` — the typed AST layer over the lossless `migjorn-syntax` CST.
//!
//! Typed views (`Surface`, `Transform`, …) are projected on demand from the
//! flat CST. They carry the token indices they were built from, so higher-level
//! edits (e.g. renumbering) can rewrite the exact tokens while the CST keeps the
//! rest of the model byte-for-byte intact.

mod cell;
mod compose;
mod datacard;
mod emit;
mod material;
mod model;
mod num;
mod renumber;
mod surface;
mod transform;

// The public API is `Model` (the facade) plus the `ModelView` its `view()`
// hands out, the owned typed views those return, and the structured
// error/diagnostic types. The typed-projection functions, the emitter, numeric
// parsing, renumbering internals, and the whole CST layer are implementation
// details and are intentionally *not* re-exported here — all capability is
// reached through `Model`/`ModelView`.
//
// The split is the editing/reading boundary: `Model` owns the edits, and
// `Model::view` materialises any pending splices once and returns a `ModelView`
// whose readers are all `&self`. Reads therefore compose and can be shared, and
// a reader can never observe a tree that disagrees with `to_source()`.
pub use cell::{Cell, CellParam, CellRef, Fill, GeomExpr, OwnedCell, SurfaceRef};
pub use compose::{ConflictKind, MergeConflict};
pub use datacard::DataCard;
pub use material::{Material, MaterialEntry};
pub use model::{EditError, Model, ModelIndex, ModelView};
pub use surface::{Surface, SurfaceKind};
pub use transform::Transform;

// Parser diagnostics are part of the public contract (see `Model::diagnostics`).
// They originate in the internal `migjorn-syntax` layer; re-export just these so
// downstream users depend only on `migjorn` and never see the CST.
pub use migjorn_syntax::{Diagnostic, Severity, Span};
