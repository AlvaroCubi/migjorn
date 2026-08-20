//! `migjorn` — a fast, lossless MCNP input parser and editor.
//!
//! Parse a model, read it as typed cells / surfaces / materials / transforms,
//! and write it back byte-for-byte identical except where you changed it.
//!
//! ```
//! let model = migjorn::Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
//! assert_eq!(model.cell(1).unwrap().material(), Some(1));
//! assert_eq!(model.surface(1).unwrap().kind(), Some("SO"));
//! ```
//!
//! The core invariants — losslessness, recoverability, no whole-file reparse on
//! any edit, and consistent reads — are described in
//! `docs/01-goals-and-invariants.md`.
//!
//! # Composing many models in one process
//!
//! `migjorn` parallelizes some of its own internal work (parsing, index
//! construction) on the global rayon pool, sized off `num_cpus` by default. A
//! caller that itself parses many models in parallel — one rayon task per
//! file, as `gitronics::load_fillers` does — ends up nesting migjorn's
//! internal fan-out inside its own, which mostly just burns kernel time
//! waking and parking threads rather than doing useful work. If your process
//! does file-level parallelism over `migjorn` calls, cap the pool once at
//! startup rather than accepting the default:
//!
//! ```no_run
//! rayon::ThreadPoolBuilder::new().num_threads(8).build_global().unwrap();
//! ```
//!
//! or set `RAYON_NUM_THREADS`. See `docs/05-parallelism-overhead.md` for the
//! measurements behind this.

mod cell;
mod compose;
mod data;
mod diagnostic;
mod edit;
mod model;
mod renumber;
mod scan;
mod surface;
mod view;

pub use cell::{CellParam, Fill, GeometryTerm, GeometryTermKind};
pub use diagnostic::{Diagnostic, Severity};
pub use edit::EditError;
pub use model::Model;
pub use view::{CellView, DataCardView, MaterialView, SurfaceView, TransformView};

pub use migjorn_syntax::{Card, CardKind, Cst, Eol, SyntaxKind, Token};

/// Parse MCNP source text.
pub fn parse(src: &str) -> Model {
    Model::parse(src)
}
