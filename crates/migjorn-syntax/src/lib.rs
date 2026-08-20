//! `migjorn-syntax` — the lossless, per-card concrete syntax layer for MCNP
//! input files.
//!
//! The one decision that drives everything: **cards are the atomic unit**. The
//! model is an ordered list of cards, and each card owns its exact bytes plus
//! the tokens indexing them. Consequences:
//!
//! * Emission is a copy of each card's text — byte-for-byte lossless when
//!   unedited, with no fast/slow path to fall off.
//! * A structural edit moves entries in a `Vec<u32>`; nothing is re-lexed.
//! * An in-card edit rewrites that one card's text and tokens. No other card is
//!   touched, ever.
//!
//! See `docs/02-architecture.md` for the rationale and the measurements
//! behind it.

mod card;
mod cst;
mod kind;
mod lex;
mod lines;
mod segment;
mod token;

pub use card::Card;
pub use cst::Cst;
pub use kind::{CardKind, Eol, SyntaxKind};
pub use lex::lex;
pub use token::Token;
