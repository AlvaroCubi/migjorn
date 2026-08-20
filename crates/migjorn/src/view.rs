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

use crate::cell::{self, CellParam, Fill, GeometryTerm, GeometryTermKind};
use crate::data::{self, DataHead};
use crate::model::Model;
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

    /// Signed surfaces in file order: `-1` keeps its sense, `+5` its prefix.
    pub fn signed_surfaces(&self) -> Vec<i64> {
        self.geometry()
            .iter()
            .filter(|t| t.kind == GeometryTermKind::Surface)
            .filter_map(|t| crate::scan::parse_int(&t.text))
            .collect()
    }

    /// Magnitudes of every surface the geometry references.
    ///
    /// Numbers inside `#( ... )` count here — that form complements a *region*
    /// of surfaces. A bare `#n` does not; see [`CellView::cell_refs`].
    pub fn surface_ids(&self) -> Vec<i64> {
        self.signed_surfaces().into_iter().map(i64::abs).collect()
    }

    /// Cells referenced by a `#n` complement, plus a `LIKE n` base.
    pub fn cell_refs(&self) -> Vec<i64> {
        let card = self.require();
        let l = cell::layout(card);
        let mut out: Vec<i64> = cell::walk_geometry(card, &l.geometry)
            .iter()
            .filter(|t| t.kind == GeometryTermKind::Complement)
            .filter_map(|t| crate::scan::parse_int(t.text.trim_start_matches('#')))
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

    pub fn universe(&self) -> Option<i64> {
        self.param("u")
            .and_then(|p| crate::scan::parse_int(p.value.trim()))
    }

    pub fn fill(&self) -> Option<Fill> {
        let card = self.require();
        let p = self
            .params()
            .into_iter()
            .find(|p| p.key.eq_ignore_ascii_case("fill"))?;
        cell::fill(card, &p)
    }

    pub fn well_formed(&self) -> bool {
        cell::layout(self.require()).well_formed
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

    pub fn well_formed(&self) -> bool {
        surface::layout(self.require()).well_formed
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

    pub fn coeffs(&self) -> Vec<f64> {
        data::values(self.require(), self.head().values_start)
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
