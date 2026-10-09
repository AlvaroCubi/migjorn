//! Borrowed, typed views over individual cards.
//!
//! A view is `(model, slot)` — two words. It projects on demand from the card's
//! own tokens, so a read always reflects current state and there is nothing to
//! flush or materialize. Reads take `&self`, so two cells can be read at once;
//! writes go through `Model` addressed by slot.
//!
//! The only way a view goes stale is if *its own* card is removed, which is
//! detected (`Model::card` returns `None`) rather than silently misread.

use migjorn_syntax::Card;

use crate::cell::{self, CellParam, Fill, GeometryTerm, ParamSpan, SurfaceRef};
use crate::data::{self, DataHead};
use crate::diagnostic::Diagnostic;
use crate::expr::{self, Expr, GeometryError};
use crate::model::Model;
use crate::param::{self, FillSpec, TransformSpec};
use crate::surface;

macro_rules! view {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy)]
        pub struct $name<'a> {
            model: &'a Model,
            slot: u32,
        }

        impl<'a> $name<'a> {
            pub(crate) fn new(model: &'a Model, slot: u32) -> Self {
                Self { model, slot }
            }

            /// The stable slot this view resolves through.
            pub fn slot(&self) -> u32 {
                self.slot
            }

            /// `None` once this card has been removed.
            pub fn card(&self) -> Option<&'a Card> {
                self.model.card(self.slot)
            }

            fn require(&self) -> &'a Card {
                self.card().expect("view refers to a removed card")
            }

            /// This card's exact current text.
            pub fn text(&self) -> &'a str {
                self.require().text()
            }

            /// The 1-based line, in [`Model::to_source`] as it reads now, of
            /// the card's first token (comment lines above it not counted).
            ///
            /// Counts the lines of every card before this one, so it costs a
            /// pass over the text up to here; to place many cards, build a
            /// [`Model::card_lines`] table once instead.
            pub fn line(&self) -> usize {
                self.model
                    .line_of(self.slot)
                    .expect("view refers to a removed card")
            }

            /// Every problem with this card as it reads now: what
            /// `Model::diagnostics` reported for it at parse, recomputed so it
            /// follows edits. Spans are relative to [`Self::text`], and `slot`
            /// is this card's. Duplicate ids are a model-wide check and only
            /// appear in `Model::diagnostics`.
            pub fn diagnostics(&self) -> Vec<Diagnostic> {
                crate::model::card_diagnostics(self.require(), self.slot)
            }
        }
    };
}

view!(CellView, "A live handle onto a cell card.");
view!(SurfaceView, "A live handle onto a surface card.");
view!(MaterialView, "A live handle onto an `Mn` material card.");
view!(TransformView, "A live handle onto a `TRn` transform card.");
view!(DataCardView, "A live handle onto any data card.");

impl<'a> CellView<'a> {
    pub fn id(&self) -> Option<i64> {
        cell::layout(self.require()).id
    }

    /// Material number; `0` means void, and then there is no density field.
    pub fn material(&self) -> Option<i64> {
        cell::layout(self.require()).material
    }

    pub fn density(&self) -> Option<f64> {
        cell::layout(self.require()).density
    }

    pub fn is_void(&self) -> bool {
        cell::layout(self.require()).material == Some(0)
    }

    /// Base cell of a `LIKE n BUT` card.
    pub fn like(&self) -> Option<i64> {
        cell::layout(self.require()).like
    }

    pub fn geometry(&self) -> Vec<GeometryTerm> {
        let card = self.require();
        cell::walk_geometry(card, &cell::layout(card).geometry)
    }

    /// The geometry expression as a tree, precedence applied (see [`Expr`]).
    ///
    /// `Err` exactly when the geometry has a diagnostic, naming the first
    /// problem and the token at fault: an unbalanced parenthesis, a `:` with
    /// no operand on one side, an empty `()`, a `#` with no cell or group
    /// after it, or a number that is not a surface or cell reference. A
    /// `LIKE n BUT` cell has no geometry of its own, and is an `Err` too.
    pub fn geometry_expr(&self) -> Result<Expr, GeometryError> {
        let card = self.require();
        expr::build(card, &cell::layout(card).geometry)
    }

    /// The geometry expression's exact source text, trimmed.
    pub fn geometry_text(&self) -> String {
        let card = self.require();
        cell::geometry_text(card, &cell::layout(card).geometry)
    }

    /// Surface references in file order, facets included: `-470.1` is
    /// `SurfaceRef { id: 470, facet: Some(1), negative: true }`. A number that
    /// is not a valid reference is left out (and the cell is not well formed).
    pub fn surface_refs(&self) -> Vec<SurfaceRef> {
        self.geometry().iter().filter_map(|t| t.surface).collect()
    }

    /// Signed surfaces in file order: `-1` keeps its sense, and a facet
    /// reference counts as its macrobody (`-470.1` -> `-470`).
    pub fn signed_surfaces(&self) -> Vec<i64> {
        self.surface_refs()
            .iter()
            .map(SurfaceRef::signed_id)
            .collect()
    }

    /// Magnitudes of every surface the geometry references.
    ///
    /// Numbers inside `#( ... )` count here — that form complements a *region*
    /// of surfaces. A bare `#n` does not; see [`CellView::cell_refs`].
    pub fn surface_ids(&self) -> Vec<i64> {
        self.surface_refs().iter().map(|r| r.id).collect()
    }

    /// Cells referenced by a `#n` complement, plus a `LIKE n` base.
    pub fn cell_refs(&self) -> Vec<i64> {
        let card = self.require();
        let l = cell::layout(card);
        let mut out: Vec<i64> = cell::walk_geometry(card, &l.geometry)
            .iter()
            .filter_map(|t| t.cell)
            .collect();
        if let Some(base) = l.like {
            out.push(base);
        }
        out
    }

    pub fn params(&self) -> Vec<CellParam> {
        let card = self.require();
        cell::params(card, &cell::layout(card).params)
    }

    /// Look a parameter up by its qualified key: `imp:n`, `vol`, `fill`.
    pub fn param(&self, key: &str) -> Option<CellParam> {
        self.params()
            .into_iter()
            .find(|p| p.qualified_key().eq_ignore_ascii_case(key))
    }

    /// The first parameter named `key` (any particle, starred or not).
    fn param_span(&self, key: &str) -> Option<ParamSpan> {
        let card = self.require();
        cell::param_spans(card, &cell::layout(card).params)
            .0
            .into_iter()
            .find(|p| p.key(card).eq_ignore_ascii_case(key))
    }

    /// Read the first `key` parameter with `read`. `None` when the parameter
    /// is absent or its value cannot be read; the latter also makes
    /// [`CellView::well_formed`] false.
    fn scalar<T>(
        &self,
        key: &str,
        read: impl FnOnce(&Card, &std::ops::Range<usize>) -> Result<T, param::ValueError>,
    ) -> Option<T> {
        let p = self.param_span(key)?;
        read(self.require(), &p.value_tokens).ok()
    }

    /// `U=`. `None` when absent or unreadable (see [`CellView::well_formed`]).
    pub fn universe(&self) -> Option<i64> {
        self.scalar("u", |c, v| param::read_int(c, "U", v))
    }

    /// `MAT=`, the material a `LIKE n BUT` cell switches to (`0` is void).
    /// `None` when absent or unreadable (see [`CellView::well_formed`]).
    pub fn material_override(&self) -> Option<i64> {
        self.scalar("mat", param::read_material)
    }

    /// `RHO=`, the density a `LIKE n BUT` cell switches to, sign as written.
    /// `None` when absent or unreadable (see [`CellView::well_formed`]).
    pub fn density_override(&self) -> Option<f64> {
        self.scalar("rho", |c, v| param::read_float(c, "RHO", v))
    }

    /// `LAT=`: `1` (hexahedral), `2` (hexagonal prism), or `0` (not a
    /// lattice; MCNP accepts it). `None` when absent, or when the value is
    /// anything else (see [`CellView::well_formed`]).
    pub fn lattice(&self) -> Option<u8> {
        self.scalar("lat", param::read_lattice)
    }

    /// The importance of `particle` (`"n"`, `"p"`, case ignored), from the
    /// first `IMP:` parameter whose particle list names it: `IMP:N=1`,
    /// `IMP:N,P=0`. `None` when no `IMP` names it or its value is unreadable
    /// (see [`CellView::well_formed`]).
    pub fn importance(&self, particle: &str) -> Option<f64> {
        let card = self.require();
        let p = cell::param_spans(card, &cell::layout(card).params)
            .0
            .into_iter()
            .find(|p| {
                p.key(card).eq_ignore_ascii_case("imp")
                    && p.particle(card)
                        .is_some_and(|list| param::names_particle(list, particle))
            })?;
        param::read_importance(card, &p.value_tokens).ok()
    }

    /// The single-universe `fill=`, kept in its text form for editing (see
    /// `Model::set_fill`). `None` for the lattice-array form; read that, and
    /// every fill as typed values, with [`CellView::fill_spec`].
    pub fn fill(&self) -> Option<Fill> {
        let card = self.require();
        let p = self
            .params()
            .into_iter()
            .find(|p| p.key.eq_ignore_ascii_case("fill"))?;
        cell::fill(card, &p)
    }

    /// The `FILL` / `*FILL` value as typed values, single or lattice array.
    /// The `bool` is the star: angles in degrees for every inline transform.
    ///
    /// Shortcuts are expanded the way MCNP 6.2 reads them: `nR` / `R`
    /// repeat the previous universe, `nI` / `I` interpolate, `nM` multiplies.
    /// A `( … )` group applies to the entry just before it (after `nR`, the
    /// last repeated one). Universes keep their sign as written.
    ///
    /// `None` when the cell has no fill. `Err` names what could not be read:
    /// an entry count that does not match the index ranges, an empty range,
    /// `nJ` / `nILOG` / bare `M` or a shortcut right after a group (all fatal
    /// in MCNP), and `nI` / `nM` that do not land on whole universe numbers
    /// (MCNP truncates them, so the shortcut does not mean what it reads as).
    pub fn fill_spec(&self) -> Option<Result<(FillSpec, bool), String>> {
        let p = self.param_span("fill")?;
        Some(
            param::read_fill(self.require(), &p.value_tokens)
                .map(|f| (f, p.starred))
                .map_err(|e| e.message),
        )
    }

    /// The `TRCL` / `*TRCL` value. The `bool` is the star: rotation entries
    /// in degrees. A group of one value is a `TRn` number, with or without
    /// parentheses; an inline transform holds 2, 3, 6, 9, 12 or 13 values,
    /// the counts MCNP accepts.
    ///
    /// `None` when the cell has no `TRCL`; `Err` names what could not be read.
    /// `TRCL=5 0 0` is an `Err`: MCNP reads it as `TR5`, not a displacement.
    pub fn trcl(&self) -> Option<Result<(TransformSpec, bool), String>> {
        let p = self.param_span("trcl")?;
        Some(
            param::read_trcl(self.require(), &p.value_tokens)
                .map(|t| (t, p.starred))
                .map_err(|e| e.message),
        )
    }

    /// Whether the card reads in full: its layout (id, material, density,
    /// geometry), every geometry reference, and the values of `FILL`, `TRCL`,
    /// `U`, `MAT`, `RHO`, `LAT` and `IMP`. When `false`,
    /// [`Self::diagnostics`] says why.
    pub fn well_formed(&self) -> bool {
        let card = self.require();
        let l = cell::layout(card);
        l.well_formed
            && cell::geometry_problems(card, &l.geometry).is_empty()
            && cell::param_problems(card, &l.params).is_empty()
    }
}

impl<'a> SurfaceView<'a> {
    pub fn id(&self) -> Option<i64> {
        surface::layout(self.require()).id
    }

    /// The surface mnemonic as written: `SO`, `PX`, `RPP`, ...
    pub fn kind(&self) -> Option<&'a str> {
        let card = self.require();
        surface::mnemonic(card, &surface::layout(card))
    }

    /// The coefficients, up to the first token that is not a number. Such a
    /// token (a shortcut like `2R` included) makes the surface not well
    /// formed, so check [`SurfaceView::well_formed`] before trusting the list.
    pub fn coeffs(&self) -> Vec<f64> {
        let card = self.require();
        surface::coeffs(card, &surface::layout(card))
    }

    /// Transform number; negative means a periodic surface.
    pub fn transform(&self) -> Option<i64> {
        surface::layout(self.require()).transform
    }

    /// Leading `*` — a reflective boundary.
    pub fn reflective(&self) -> bool {
        surface::layout(self.require()).reflective
    }

    /// Leading `+` — a white boundary.
    pub fn white(&self) -> bool {
        surface::layout(self.require()).white
    }

    /// Whether the card reads in full: id, mnemonic, at least one
    /// coefficient, and every coefficient a number.
    pub fn well_formed(&self) -> bool {
        let card = self.require();
        let l = surface::layout(card);
        l.well_formed && surface::coeff_problem(card, &l).is_none()
    }
}

impl<'a> MaterialView<'a> {
    fn head(&self) -> DataHead {
        data::head(self.require()).expect("material card has a head")
    }

    pub fn id(&self) -> Option<i64> {
        data::material_id(&self.head())
    }

    /// `(zaid, fraction)` pairs. The ZAID keeps its library suffix; a negative
    /// fraction is by weight.
    pub fn entries(&self) -> Vec<(String, f64)> {
        let card = self.require();
        data::material_entries(card, &self.head()).0
    }

    pub fn well_formed(&self) -> bool {
        let card = self.require();
        data::material_entries(card, &self.head()).1
    }
}

impl<'a> TransformView<'a> {
    fn head(&self) -> DataHead {
        data::head(self.require()).expect("transform card has a head")
    }

    pub fn id(&self) -> Option<i64> {
        data::transform_id(&self.head())
    }

    /// `*TRn`: the rotation entries are angles in degrees.
    pub fn degrees(&self) -> bool {
        self.head().starred
    }

    /// The values, up to the first token that is not a number. Trailing `nJ`
    /// jumps are values left off, so this is the full list; any other such
    /// token makes the transform not well formed, so check
    /// [`TransformView::well_formed`] before trusting the list.
    pub fn coeffs(&self) -> Vec<f64> {
        data::values(self.require(), self.head().values_start)
    }

    /// Whether MCNP reads the card as written: every value a number, and 0,
    /// 1, 2, 3, 6, 9, 12 or 13 of them. Trailing `nJ` jumps count as values
    /// left off; any other shortcut makes the card not well formed.
    pub fn well_formed(&self) -> bool {
        data::transform_problem(self.require(), &self.head()).is_none()
    }

    /// The first three coefficients — the origin displacement.
    pub fn displacement(&self) -> Vec<f64> {
        self.coeffs().into_iter().take(3).collect()
    }
}

impl<'a> DataCardView<'a> {
    fn head(&self) -> Option<DataHead> {
        data::head(self.require())
    }

    /// The card's name as written, id included: `m1`, `f4`, `sdef`.
    pub fn name(&self) -> Option<&'a str> {
        let card = self.require();
        let head = data::head(card)?;
        Some(card.token_text(head.name_tok))
    }

    pub fn particle(&self) -> Option<String> {
        self.head()?.particle
    }

    pub fn starred(&self) -> bool {
        self.head().is_some_and(|h| h.starred)
    }
}
