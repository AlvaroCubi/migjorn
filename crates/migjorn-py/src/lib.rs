//! Python bindings for the migjorn MCNP parser.
//!
//! The public surface mirrors `migjorn`'s `Model` facade with Pythonic
//! ergonomics: `migjorn.parse(text)` (or `migjorn.Model.from_file(path)`) returns
//! a `Model` exposing typed access, id lookups, lossless re-emission, and
//! whole-geometry renumbering.
//!
//! The typed objects (`Cell`, `Surface`, `Material`, `Transform`) are **live
//! handles**, not snapshots: each holds a reference to its `Model` plus a stable
//! card slot. Getters read the current card on demand and setters (e.g.
//! `cell.material = 124`) write straight through the lossless editing engine, so
//! edits are visible immediately and the rest of the model stays byte-for-byte.

use std::cell::OnceCell;

use migjorn as core;
use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rustc_hash::FxHashMap;

/// Error raised when a handle's card no longer exists (it was deleted, or the
/// model was structurally changed out from under it).
fn stale_handle() -> PyErr {
    PyRuntimeError::new_err("stale handle: the referenced card no longer exists")
}

/// Map a core editing error (an invalid or unsupported edit — e.g. emptying a
/// cell's geometry, or an out-of-range index) to a Python `ValueError`.
fn edit_error(e: core::EditError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

pyo3::create_exception!(
    _migjorn,
    MergeError,
    PyValueError,
    "Raised by `Model.merge` when the merged models share a cell, surface, \
     material, or transform id. A subclass of `ValueError`."
);

/// Map merge id-collisions to a `MergeError` whose message lists every conflict.
fn merge_error(conflicts: &[core::MergeConflict]) -> PyErr {
    let msg = conflicts
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    MergeError::new_err(format!("cannot merge: {msg}"))
}

/// A cell card (a live handle onto its model).
///
/// Attributes:
///     id (int): Cell number.
///     material (int | None): Material number (0 = void; None for ``LIKE n BUT``).
///         Assignable in place. Assigning a real material to a void cell gives
///         it a placeholder density of 0.0 (set the real value via ``density``);
///         assigning 0 makes the cell void and drops its density.
///     density (float | None): Density (positive = atom, negative = mass).
///         Assignable in place for a non-void cell.
///     is_void (bool): True when the material number is 0.
///     like (int | None): Base cell for a ``LIKE n BUT`` card.
///     surface_ids (list[int]): Referenced surface numbers (magnitudes).
///     signed_surfaces (list[int]): Referenced surfaces with sense (sign).
///     cell_refs (list[int]): Referenced cells (``#n`` complements, ``LIKE n``).
///     universe (int | None): The cell's ``u=`` universe, or None if unset.
///     fill (Fill | None): The cell's ``fill=`` parameter (simple form), or None.
///     well_formed (bool): False if the geometry could not be fully parsed.
///     text (str): The card's exact source (incl. inline ``$`` comments),
///         reflecting any edits.
#[pyclass(module = "migjorn")]
struct Cell {
    model: Py<Model>,
    slot: u32,
}

impl Cell {
    /// Take a [`ModelView`] over the model, resolve this handle's current card
    /// index, and run `f`. `borrow_mut` because `Model::view` materialises any
    /// pending splices so reads match emission; the borrow is released before
    /// this returns, since every read hands back owned data.
    ///
    /// [`ModelView`]: core::ModelView
    fn with_view<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&core::ModelView, usize) -> PyResult<R>,
    ) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let view = m.inner.view();
        let ci = view
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        f(&view, ci)
    }

    /// Resolve this handle's current card and read it, running `f` on it.
    /// Raises if the card no longer exists or is no longer a cell.
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Cell) -> R) -> PyResult<R> {
        self.with_view(py, |view, ci| {
            let cell = view
                .cell_at(ci)
                .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid cell"))?;
            Ok(f(&cell))
        })
    }

    /// Run a mutation `f` on the model for this handle's card, then invalidate
    /// the id index.
    fn edit<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut core::Model, usize) -> Result<R, core::EditError>,
    ) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        let r = f(&mut m.inner, ci).map_err(edit_error)?;
        m.invalidate();
        Ok(r)
    }
}

#[pymethods]
impl Cell {
    #[getter]
    fn id(&self, py: Python<'_>) -> PyResult<i64> {
        self.read(py, |c| c.id)
    }
    #[getter]
    fn material(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.read(py, |c| c.material)
    }
    #[setter]
    fn set_material(&self, py: Python<'_>, value: i64) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_cell_material(ci, value))
    }
    #[getter]
    fn density(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        self.read(py, |c| c.density)
    }
    #[setter]
    fn set_density(&self, py: Python<'_>, value: f64) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_cell_density(ci, value))
    }
    #[getter]
    fn is_void(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |c| c.is_void())
    }
    #[getter]
    fn like(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.read(py, |c| c.like_id())
    }
    #[getter]
    fn surface_ids(&self, py: Python<'_>) -> PyResult<Vec<i64>> {
        self.read(py, |c| c.surface_ids())
    }
    #[getter]
    fn signed_surfaces(&self, py: Python<'_>) -> PyResult<Vec<i64>> {
        self.read(py, |c| c.signed_surfaces())
    }
    #[getter]
    fn cell_refs(&self, py: Python<'_>) -> PyResult<Vec<i64>> {
        self.read(py, |c| c.cell_ref_ids())
    }
    #[getter]
    fn universe(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.with_view(py, |view, ci| Ok(view.cell_universe(ci)))
    }
    #[getter]
    fn fill(&self, py: Python<'_>) -> PyResult<Option<Fill>> {
        self.with_view(py, |view, ci| Ok(view.cell_fill(ci).map(Fill::from)))
    }
    #[getter]
    fn well_formed(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |c| c.well_formed)
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        Ok(m.inner.card_source(ci))
    }

    /// Intersect the geometry with a signed surface (negative int = negative
    /// sense). Spliced in losslessly; the rest of the card stays byte-for-byte.
    fn add_surface(&self, py: Python<'_>, surface: i64) -> PyResult<()> {
        self.edit(py, |m, ci| {
            m.add_cell_surface(ci, surface.abs(), surface < 0)
        })
    }

    /// Remove every reference to surface `id` (either sense) from the geometry.
    /// Returns whether anything was removed; raises if it would empty the cell.
    fn remove_surface(&self, py: Python<'_>, id: i64) -> PyResult<bool> {
        self.edit(py, |m, ci| m.remove_cell_surface(ci, id.abs()))
    }

    /// Intersect the geometry with a `#n` complement of cell `id`.
    fn add_complement(&self, py: Python<'_>, id: i64) -> PyResult<()> {
        self.edit(py, |m, ci| m.add_cell_complement(ci, id))
    }

    /// Remove every `#n` complement of cell `id` from the geometry.
    fn remove_complement(&self, py: Python<'_>, id: i64) -> PyResult<bool> {
        self.edit(py, |m, ci| m.remove_cell_complement(ci, id))
    }

    #[getter]
    fn params(&self, py: Python<'_>) -> PyResult<Vec<CellParam>> {
        self.with_view(py, |view, ci| {
            Ok(view.cell_params(ci).into_iter().map(Into::into).collect())
        })
    }

    /// Read the first parameter matching ``key`` — a bare keyword (``"vol"``) or
    /// a particle-qualified one (``"imp:n"``) — or ``None`` if absent.
    fn param(&self, py: Python<'_>, key: &str) -> PyResult<Option<CellParam>> {
        self.with_view(py, |view, ci| Ok(view.cell_param(ci, key).map(Into::into)))
    }

    /// Rewrite the value of the first parameter matching ``key`` in place (e.g.
    /// ``set_param("imp:n", "2")``), returning whether one matched. Only the
    /// value changes — the keyword and its position are preserved, so unlike
    /// remove-then-add the card stays byte-for-byte apart from the new value.
    fn set_param(&self, py: Python<'_>, key: &str, value: &str) -> PyResult<bool> {
        self.edit(py, |m, ci| m.set_cell_param(ci, key, value))
    }

    /// Append a parameter (e.g. ``"imp:n=1"`` or ``"u=5"``) to the cell's
    /// parameter section. Spliced in after the cell's last token and before any
    /// trailing inline ``$`` comment, so the rest of the card is preserved.
    fn add_param(&self, py: Python<'_>, text: &str) -> PyResult<()> {
        self.edit(py, |m, ci| m.add_cell_param(ci, text))
    }

    /// Remove the first parameter matching ``key``. A bare keyword (``"imp"``)
    /// matches the first entry with that keyword regardless of designator; a
    /// qualified key (``"imp:n"``) matches only that particle, so ``imp:n`` and
    /// ``imp:p`` are individually removable. Case-insensitive. Returns whether
    /// one was removed.
    fn remove_param(&self, py: Python<'_>, key: &str) -> PyResult<bool> {
        self.edit(py, |m, ci| m.remove_cell_param(ci, key))
    }

    /// Append ``$ text`` as an inline comment after the cell's last token.
    /// Lossless: every other byte of the card is preserved.
    fn append_comment(&self, py: Python<'_>, text: &str) -> PyResult<()> {
        self.edit(py, |m, ci| m.append_inline_comment(ci, text))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |c| {
            format!(
                "Cell(id={}, material={:?}, density={:?})",
                c.id, c.material, c.density
            )
        })
    }
}

/// A surface card (a live handle onto its model).
///
/// Attributes:
///     id (int): Surface number.
///     kind (str): MCNP mnemonic, e.g. ``"PX"``, ``"GQ"``, ``"C/X"``, ``"RPP"``.
///     coeffs (list[float]): Surface coefficients.
///     transform (int | None): Transformation number (negative => periodic).
///     reflective (bool): Reflective boundary (leading ``*``).
///     white (bool): White boundary (leading ``+``).
///     well_formed (bool): False if a coefficient could not be parsed.
///     text (str): The card's exact source, reflecting any edits.
#[pyclass(module = "migjorn")]
struct Surface {
    model: Py<Model>,
    slot: u32,
}

impl Surface {
    /// Take a view (materialising pending splices so reads match emission) and
    /// read this handle's card. Raises if the card no longer exists or is no
    /// longer a surface.
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Surface) -> R) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let view = m.inner.view();
        if view.card_index_of_slot(self.slot).is_none() {
            return Err(stale_handle());
        }
        let surface = view
            .surface_by_slot(self.slot)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid surface"))?;
        Ok(f(&surface))
    }

    /// Run a mutation `f` on the model for this handle's card, then invalidate
    /// the id index.
    fn edit<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut core::Model, usize) -> Result<R, core::EditError>,
    ) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        let r = f(&mut m.inner, ci).map_err(edit_error)?;
        m.invalidate();
        Ok(r)
    }
}

#[pymethods]
impl Surface {
    #[getter]
    fn id(&self, py: Python<'_>) -> PyResult<i64> {
        self.read(py, |s| s.id)
    }
    #[getter]
    fn kind(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |s| s.kind.mnemonic())
    }
    #[getter]
    fn coeffs(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.read(py, |s| s.coeffs.clone())
    }
    /// Rewrite every coefficient in place. The number of values must match the
    /// current coefficient count (changing it is a structural edit).
    #[setter]
    fn set_coeffs(&self, py: Python<'_>, values: Vec<f64>) -> PyResult<()> {
        let current = self.read(py, |s| s.coeffs.len())?;
        if values.len() != current {
            return Err(PyValueError::new_err(format!(
                "expected {current} coefficient(s), got {}",
                values.len()
            )));
        }
        let mut m = self.model.bind(py).borrow_mut();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        for (i, v) in values.iter().enumerate() {
            m.inner.set_surface_coeff(ci, i, *v).map_err(edit_error)?;
        }
        m.invalidate();
        Ok(())
    }
    #[getter]
    fn transform(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.read(py, |s| s.transform)
    }
    /// Change the transformation number in place (negative => periodic). Adding
    /// a transform to a surface that has none, or removing an existing one, is a
    /// lossless splice; every other byte of the card is preserved.
    #[setter]
    fn set_transform(&self, py: Python<'_>, value: Option<i64>) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_surface_transform(ci, value))
    }
    #[getter]
    fn reflective(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |s| s.reflective)
    }
    #[getter]
    fn white(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |s| s.white)
    }
    #[getter]
    fn well_formed(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |s| s.well_formed)
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        Ok(m.inner.card_source(ci))
    }

    /// Set a single coefficient (by index) in place. Raises ``ValueError`` for
    /// an out-of-range index.
    fn set_coeff(&self, py: Python<'_>, index: usize, value: f64) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_surface_coeff(ci, index, value))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |s| {
            format!(
                "Surface(id={}, kind={:?}, coeffs={:?})",
                s.id,
                s.kind.mnemonic(),
                s.coeffs
            )
        })
    }
}

/// A material (``Mn``) card (a live handle onto its model).
///
/// Attributes:
///     id (int): Material number.
///     entries (list[tuple[str, float]]): ``(zaid, fraction)`` pairs; a
///         positive fraction is atomic, a negative fraction is by weight.
///     well_formed (bool): False if entries were not clean ZAID/fraction pairs.
///     text (str): The card's exact source, reflecting any edits.
#[pyclass(module = "migjorn")]
struct Material {
    model: Py<Model>,
    slot: u32,
}

impl Material {
    /// Take a view (materialising pending splices so reads match emission) and
    /// read this handle's card. Raises if the card no longer exists or is no
    /// longer a material.
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Material) -> R) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let view = m.inner.view();
        if view.card_index_of_slot(self.slot).is_none() {
            return Err(stale_handle());
        }
        let material = view
            .material_by_slot(self.slot)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid material"))?;
        Ok(f(&material))
    }

    /// Run a mutation `f` on the model for this handle's card, then invalidate
    /// the id index.
    fn edit<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut core::Model, usize) -> Result<R, core::EditError>,
    ) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        let r = f(&mut m.inner, ci).map_err(edit_error)?;
        m.invalidate();
        Ok(r)
    }
}

#[pymethods]
impl Material {
    #[getter]
    fn id(&self, py: Python<'_>) -> PyResult<i64> {
        self.read(py, |m| m.id)
    }
    #[getter]
    fn entries(&self, py: Python<'_>) -> PyResult<Vec<(String, f64)>> {
        self.read(py, |m| {
            m.entries
                .iter()
                .map(|e| (e.zaid.clone(), e.fraction))
                .collect()
        })
    }
    #[getter]
    fn well_formed(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |m| m.well_formed)
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        Ok(m.inner.card_source(ci))
    }

    /// Set the fraction of the ``entry``-th ``(zaid, fraction)`` pair in place
    /// (positive = atomic, negative = by weight). Raises ``ValueError`` for an
    /// out-of-range index.
    fn set_fraction(&self, py: Python<'_>, entry: usize, value: f64) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_material_fraction(ci, entry, value))
    }

    /// Set the ZAID of the ``entry``-th pair in place (e.g. ``"1001.31c"``).
    fn set_zaid(&self, py: Python<'_>, entry: usize, zaid: &str) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_material_zaid(ci, entry, zaid))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |m| {
            format!("Material(id={}, entries={})", m.id, m.entries.len())
        })
    }
}

/// A coordinate transformation (``TRn`` / ``*TRn``) card (a live handle).
///
/// Attributes:
///     id (int): Transformation number.
///     degrees (bool): Rotation entries are angles in degrees (``*TRn``).
///     displacement (tuple[float, float, float]): Origin displacement.
///     rotation (list[float]): Rotation entries as written.
///     text (str): The card's exact source, reflecting any edits.
#[pyclass(module = "migjorn")]
struct Transform {
    model: Py<Model>,
    slot: u32,
}

impl Transform {
    /// Take a view (materialising pending splices so reads match emission) and
    /// read this handle's card. Raises if the card no longer exists or is no
    /// longer a transform.
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Transform) -> R) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let view = m.inner.view();
        if view.card_index_of_slot(self.slot).is_none() {
            return Err(stale_handle());
        }
        let transform = view
            .transform_by_slot(self.slot)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid transform"))?;
        Ok(f(&transform))
    }

    /// Run a mutation `f` on the model for this handle's card, then invalidate
    /// the id index.
    fn edit<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut core::Model, usize) -> Result<R, core::EditError>,
    ) -> PyResult<R> {
        let mut m = self.model.bind(py).borrow_mut();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        let r = f(&mut m.inner, ci).map_err(edit_error)?;
        m.invalidate();
        Ok(r)
    }
}

#[pymethods]
impl Transform {
    #[getter]
    fn id(&self, py: Python<'_>) -> PyResult<i64> {
        self.read(py, |t| t.id)
    }
    #[getter]
    fn degrees(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |t| t.degrees)
    }
    #[getter]
    fn displacement(&self, py: Python<'_>) -> PyResult<(f64, f64, f64)> {
        self.read(py, |t| {
            (t.displacement[0], t.displacement[1], t.displacement[2])
        })
    }
    /// Set the displacement vector in place. Components already written are
    /// rewritten; a component that defaulted to 0 (no token) is spliced in
    /// losslessly after the last present value.
    #[setter]
    fn set_displacement(&self, py: Python<'_>, value: (f64, f64, f64)) -> PyResult<()> {
        let d = [value.0, value.1, value.2];
        self.edit(py, |m, ci| m.set_transform_displacement(ci, d))
    }
    #[getter]
    fn rotation(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.read(py, |t| t.rotation.clone())
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let ci = m
            .inner
            .card_index_of_slot(self.slot)
            .ok_or_else(stale_handle)?;
        Ok(m.inner.card_source(ci))
    }

    /// Rewrite the rotation entries in place. Extra entries are spliced in and
    /// surplus entries deleted losslessly; adding rotation to a transform whose
    /// displacement is incomplete is ambiguous and raises ``ValueError``.
    fn set_rotation(&self, py: Python<'_>, rotation: Vec<f64>) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_transform_rotation(ci, &rotation))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |t| {
            format!("Transform(id={}, degrees={})", t.id, t.degrees)
        })
    }
}

/// A cell's ``fill=`` parameter (simple single-universe form).
///
/// Attributes:
///     universe (int): The universe number filling the cell.
///     starred (bool): True when written ``*fill`` (transform angles in degrees).
///     transform (str | None): Raw text inside the ``fill= u (...)`` parentheses
///         -- a ``TRn`` reference or an inline transform list -- or None.
#[pyclass(frozen, module = "migjorn")]
struct Fill {
    #[pyo3(get)]
    universe: i64,
    #[pyo3(get)]
    starred: bool,
    #[pyo3(get)]
    transform: Option<String>,
}

impl From<core::Fill> for Fill {
    fn from(f: core::Fill) -> Self {
        Fill {
            universe: f.universe,
            starred: f.starred,
            transform: f.transform,
        }
    }
}

#[pymethods]
impl Fill {
    fn __repr__(&self) -> String {
        format!(
            "Fill(universe={}, starred={}, transform={:?})",
            self.universe, self.starred, self.transform
        )
    }
}

/// One cell keyword parameter (``imp:n``, ``vol``, ``fill``, ``trcl``, ...) as
/// read from a card.
///
/// ``particle`` and ``starred`` are only meaningful for the parameters whose
/// grammar uses them (``:designator`` on ``imp``/``ext``/...; ``*`` on
/// ``fill``/``trcl``); for every other parameter they are ``None``/``False``.
///
/// Attributes:
///     key (str): Uppercased keyword without the designator (``"IMP"``, ``"VOL"``).
///     particle (str | None): Uppercased ``:particle`` designator (``"N"``) or None.
///     starred (bool): True when written with a ``*`` prefix (``*fill``, ``*trcl``).
///     value (str): Value text, tokens joined by single spaces (``"7 ( 0 0 5 )"``).
#[pyclass(frozen, module = "migjorn")]
struct CellParam {
    #[pyo3(get)]
    key: String,
    #[pyo3(get)]
    particle: Option<String>,
    #[pyo3(get)]
    starred: bool,
    #[pyo3(get)]
    value: String,
}

impl From<core::CellParam> for CellParam {
    fn from(p: core::CellParam) -> Self {
        CellParam {
            key: p.key,
            particle: p.particle,
            starred: p.starred,
            value: p.value,
        }
    }
}

#[pymethods]
impl CellParam {
    fn __repr__(&self) -> String {
        let particle = match &self.particle {
            Some(p) => format!(":{p}"),
            None => String::new(),
        };
        let star = if self.starred { "*" } else { "" };
        format!("CellParam({star}{}{particle}={:?})", self.key, self.value)
    }
}

/// A generically-parsed data card (anything not modelled more specifically).
///
/// Attributes:
///     name (str): Uppercased mnemonic including any number (``"SDEF"``, ``"F4"``).
///     particle (str | None): Particle designator after ``:`` (``"n"``, ``"n,p"``).
///     starred (bool): Leading ``*`` modifier (``*F``, ``*TR``).
#[pyclass(frozen, module = "migjorn")]
#[derive(Clone)]
struct DataCard {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    particle: Option<String>,
    #[pyo3(get)]
    starred: bool,
}

impl From<core::DataCard> for DataCard {
    fn from(d: core::DataCard) -> Self {
        DataCard {
            name: d.name,
            particle: d.particle,
            starred: d.starred,
        }
    }
}

#[pymethods]
impl DataCard {
    fn __repr__(&self) -> String {
        match &self.particle {
            Some(p) => format!("DataCard(name={:?}, particle={:?})", self.name, p),
            None => format!("DataCard(name={:?})", self.name),
        }
    }
}

/// A parser diagnostic.
///
/// Attributes:
///     severity (str): ``"error"`` or ``"warning"``.
///     message (str): Human-readable description.
///     start (int): Byte offset where the span begins.
///     end (int): Byte offset where the span ends.
#[pyclass(frozen, module = "migjorn")]
#[derive(Clone)]
struct Diagnostic {
    #[pyo3(get)]
    severity: String,
    #[pyo3(get)]
    message: String,
    #[pyo3(get)]
    start: u32,
    #[pyo3(get)]
    end: u32,
}

impl From<&core::Diagnostic> for Diagnostic {
    fn from(d: &core::Diagnostic) -> Self {
        let severity = match d.severity {
            core::Severity::Error => "error",
            core::Severity::Warning => "warning",
            _ => "unknown",
        };
        Diagnostic {
            severity: severity.to_string(),
            message: d.message.clone(),
            start: d.span.start,
            end: d.span.end,
        }
    }
}

#[pymethods]
impl Diagnostic {
    fn __repr__(&self) -> String {
        format!(
            "Diagnostic(severity={:?}, message={:?}, span={}..{})",
            self.severity, self.message, self.start, self.end
        )
    }
}

/// A parsed MCNP model.
///
/// Construct with :func:`migjorn.parse`, ``Model(text)``, or
/// :meth:`Model.from_file`. The model is *lossless*: :meth:`to_source` (and
/// ``str(model)``) reproduce the input byte-for-byte until you edit it, and edits
/// such as :meth:`renumber_surfaces` or ``cell.material = ...`` change only the
/// affected tokens.
///
/// The ``cells``/``surfaces``/``materials``/``transforms``/``data_cards``
/// properties materialise handle lists -- convenient, but for very large models
/// prefer the id lookups (:meth:`surface`, :meth:`cell`, ...) and the ``num_*``
/// counts, which do not build the whole list.
#[pyclass(unsendable, module = "migjorn")]
struct Model {
    inner: core::Model,
    index: OnceCell<core::ModelIndex>,
}

impl Model {
    fn build(inner: core::Model) -> Self {
        Model {
            inner,
            index: OnceCell::new(),
        }
    }

    /// The cached id index, built on first use. `&mut self`: the index is built
    /// through a `ModelView`, which materialises pending splices — so a cached
    /// index can never disagree with what the model emits. Cleared by
    /// `invalidate` after any structural/number edit.
    fn idx(&mut self) -> &core::ModelIndex {
        if self.index.get().is_none() {
            let built = self.inner.view().index();
            let _ = self.index.set(built);
        }
        self.index.get().expect("just initialised")
    }

    /// Invalidate the cached id index after a structural/number edit.
    fn invalidate(&mut self) {
        self.index = OnceCell::new();
    }
}

#[pymethods]
impl Model {
    /// Parse MCNP `text` into a model.
    #[new]
    fn new(text: &str) -> Self {
        Model::build(core::Model::parse(text))
    }

    /// Parse the MCNP model at `path`.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| PyIOError::new_err(format!("cannot read {path}: {e}")))?;
        Ok(Model::build(core::Model::parse(text)))
    }

    /// Re-emit the model as MCNP text (byte-for-byte identical to the input when
    /// unedited; only edited tokens differ otherwise).
    fn to_source(&self) -> String {
        self.inner.to_source()
    }

    /// Write the model to `path`.
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.inner.to_source())
            .map_err(|e| PyIOError::new_err(format!("cannot write {path}: {e}")))
    }

    fn __str__(&self) -> String {
        self.inner.to_source()
    }

    fn __repr__(&mut self) -> String {
        let idx = self.idx();
        format!(
            "Model(cells={}, surfaces={}, materials={}, transforms={}, diagnostics={})",
            idx.cell_count(),
            idx.surface_count(),
            idx.material_count(),
            idx.transform_count(),
            self.inner.diagnostics().len(),
        )
    }

    /// Parser diagnostics (empty on a clean parse).
    #[getter]
    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.inner
            .diagnostics()
            .iter()
            .map(Diagnostic::from)
            .collect()
    }

    /// All cells, in source order (materialises a list of handles).
    ///
    /// Built from `cell_slots()`, not `Model::view()`: card add/remove always
    /// reparses immediately and in-card splices never change a card's kind or
    /// count, so this classification can't disagree with a materialised read
    /// (see the "sanctioned" stable-slot exception in `migjorn::Model`).
    #[getter]
    fn cells(slf: Bound<'_, Self>) -> Vec<Cell> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        m.inner
            .cell_slots()
            .into_iter()
            .map(|slot| Cell {
                model: model_py.clone_ref(py),
                slot,
            })
            .collect()
    }

    /// All surfaces, in source order (materialises a list of handles).
    ///
    /// See `Model::cells` above: `surface_slots()` is splice-safe, no `view()` needed.
    #[getter]
    fn surfaces(slf: Bound<'_, Self>) -> Vec<Surface> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        m.inner
            .surface_slots()
            .into_iter()
            .map(|slot| Surface {
                model: model_py.clone_ref(py),
                slot,
            })
            .collect()
    }

    /// All ``TRn`` transforms, in source order.
    ///
    /// See `Model::cells` above: `transform_slots()` is splice-safe, no `view()` needed.
    #[getter]
    fn transforms(slf: Bound<'_, Self>) -> Vec<Transform> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        m.inner
            .transform_slots()
            .into_iter()
            .map(|slot| Transform {
                model: model_py.clone_ref(py),
                slot,
            })
            .collect()
    }

    /// All ``Mn`` materials, in source order.
    ///
    /// See `Model::cells` above: `material_slots()` is splice-safe, no `view()` needed.
    #[getter]
    fn materials(slf: Bound<'_, Self>) -> Vec<Material> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        m.inner
            .material_slots()
            .into_iter()
            .map(|slot| Material {
                model: model_py.clone_ref(py),
                slot,
            })
            .collect()
    }

    /// All data cards (generic view), in source order.
    #[getter]
    fn data_cards(&mut self) -> Vec<DataCard> {
        self.inner.view().data_cards().map(DataCard::from).collect()
    }

    /// Number of cells (cheap; does not build the cell list).
    #[getter]
    fn num_cells(&mut self) -> usize {
        self.idx().cell_count()
    }

    /// Number of surfaces (cheap; does not build the surface list).
    #[getter]
    fn num_surfaces(&mut self) -> usize {
        self.idx().surface_count()
    }

    /// Number of materials (cheap; does not build the material list).
    #[getter]
    fn num_materials(&mut self) -> usize {
        self.idx().material_count()
    }

    /// Number of transforms (cheap; does not build the transform list).
    #[getter]
    fn num_transforms(&mut self) -> usize {
        self.idx().transform_count()
    }

    /// Look up a surface by number, or ``None``.
    fn surface(slf: Bound<'_, Self>, id: i64) -> Option<Surface> {
        let mut m = slf.borrow_mut();
        let ci = m.idx().surface(id)?;
        let slot = m.inner.slot_at(ci);
        Some(Surface {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a cell by number, or ``None``.
    fn cell(slf: Bound<'_, Self>, id: i64) -> Option<Cell> {
        let mut m = slf.borrow_mut();
        let ci = m.idx().cell(id)?;
        let slot = m.inner.slot_at(ci);
        Some(Cell {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a material by number, or ``None``.
    fn material(slf: Bound<'_, Self>, id: i64) -> Option<Material> {
        let mut m = slf.borrow_mut();
        let ci = m.idx().material(id)?;
        let slot = m.inner.slot_at(ci);
        Some(Material {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a transform by number, or ``None``.
    fn transform(slf: Bound<'_, Self>, id: i64) -> Option<Transform> {
        let mut m = slf.borrow_mut();
        let ci = m.idx().transform(id)?;
        let slot = m.inner.slot_at(ci);
        Some(Transform {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Renumber every surface -- definitions **and** all references in cell
    /// geometry -- consistently.
    ///
    /// `mapping` may be either:
    ///   * a ``dict[int, int]`` (old number -> new number; unmapped numbers are
    ///     left unchanged), or
    ///   * a callable ``int -> int`` (invoked once per distinct surface number).
    fn renumber_surfaces(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_surfaces(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Renumber every cell -- definitions, ``#n`` complements, and ``LIKE n``
    /// bases -- consistently. `mapping` is a dict or callable, as for
    /// :meth:`renumber_surfaces`.
    fn renumber_cells(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_cells(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Renumber every material -- `Mn` definitions, every cell's material field,
    /// and `MTn`/`MXn` cards -- consistently. Void cells (material 0) are left
    /// unchanged. `mapping` is a dict or callable, as for
    /// :meth:`renumber_surfaces`.
    fn renumber_materials(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_materials(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Renumber every transform -- `TRn`/`*TRn` definitions and every surface's
    /// transform field (periodic sign preserved) -- consistently. `mapping` is a
    /// dict or callable, as for :meth:`renumber_surfaces`.
    fn renumber_transforms(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_transforms(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Renumber every universe -- `u=` definitions and `fill=` references
    /// (including lattice fill arrays) -- consistently. Universe 0 is left
    /// unchanged. `mapping` is a dict or callable, as for :meth:`renumber_surfaces`.
    fn renumber_universes(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_universes(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Renumber every tally *id* (`Fn` and companion cards like `E`, `FM`,
    /// `SD`, ...) via `mapping`. The cell/surface ids inside tally bins are
    /// updated by :meth:`renumber_cells` / :meth:`renumber_surfaces`, not here.
    /// `mapping` is a dict or callable, as for :meth:`renumber_surfaces`.
    fn renumber_tallies(&mut self, mapping: Bound<'_, PyAny>) -> PyResult<()> {
        let mut mapper = Mapper::build(mapping)?;
        self.inner.renumber_tallies(|id| mapper.map(id));
        mapper.into_result()?;
        self.invalidate();
        Ok(())
    }

    /// Shift every surface number by `delta` (a fast convenience for
    /// ``renumber_surfaces(lambda n: n + delta)``).
    fn offset_surfaces(&mut self, delta: i64) {
        self.inner.renumber_surfaces(|id| id + delta);
        self.invalidate();
    }

    /// Shift every cell number by `delta`.
    fn offset_cells(&mut self, delta: i64) {
        self.inner.renumber_cells(|id| id + delta);
        self.invalidate();
    }

    /// Add a new cell from MCNP `text` (a cell card body, e.g.
    /// ``"10 6 -7.85 -5 6 imp:n=1"``), appended to the cell block. Returns a live
    /// handle to the new cell. Raises ``ValueError`` if the text is not exactly
    /// one well-formed cell.
    fn add_cell(slf: Bound<'_, Self>, text: &str) -> PyResult<Cell> {
        let slot = {
            let mut m = slf.borrow_mut();
            let s = m.inner.add_cell(text).map_err(edit_error)?;
            m.invalidate();
            s
        };
        Ok(Cell {
            model: slf.unbind(),
            slot,
        })
    }

    /// Add a new surface from MCNP `text` (e.g. ``"5 SO 12.0"``), appended to the
    /// surface block. Returns a live handle to the new surface.
    fn add_surface(slf: Bound<'_, Self>, text: &str) -> PyResult<Surface> {
        let slot = {
            let mut m = slf.borrow_mut();
            let s = m.inner.add_surface(text).map_err(edit_error)?;
            m.invalidate();
            s
        };
        Ok(Surface {
            model: slf.unbind(),
            slot,
        })
    }

    /// Add a new material from MCNP `text` (e.g. ``"m7 26000 -1.0"``), appended to
    /// the data block. Returns a live handle to the new material.
    fn add_material(slf: Bound<'_, Self>, text: &str) -> PyResult<Material> {
        let slot = {
            let mut m = slf.borrow_mut();
            let s = m.inner.add_data_card(text).map_err(edit_error)?;
            m.invalidate();
            s
        };
        Ok(Material {
            model: slf.unbind(),
            slot,
        })
    }

    /// Remove the cell numbered `id`. Returns whether a cell was removed.
    fn remove_cell(&mut self, id: i64) -> PyResult<bool> {
        let removed = self.inner.remove_cell(id).map_err(edit_error)?;
        self.invalidate();
        Ok(removed)
    }

    /// Remove the surface numbered `id`. Returns whether a surface was removed.
    fn remove_surface(&mut self, id: i64) -> PyResult<bool> {
        let removed = self.inner.remove_surface(id).map_err(edit_error)?;
        self.invalidate();
        Ok(removed)
    }

    /// Remove the ``Mn`` material numbered `id`. Returns whether one was removed.
    /// Deletes the card only; a cell still pointing at it is reported by
    /// :meth:`validate`.
    fn remove_material(&mut self, id: i64) -> PyResult<bool> {
        let removed = self.inner.remove_material(id).map_err(edit_error)?;
        self.invalidate();
        Ok(removed)
    }

    /// Remove the ``TRn``/``*TRn`` transform numbered `id`. Returns whether one
    /// was removed. Deletes the card only (see :meth:`remove_material`).
    fn remove_transform(&mut self, id: i64) -> PyResult<bool> {
        let removed = self.inner.remove_transform(id).map_err(edit_error)?;
        self.invalidate();
        Ok(removed)
    }

    /// Check model consistency: return a list of human-readable problems,
    /// empty when consistent. Reports duplicate cell/surface/material/transform
    /// definitions, dangling surface/cell/material references from cells, and a
    /// surface whose transform (or periodic partner surface) is undefined.
    fn validate(&mut self) -> Vec<String> {
        self.inner.view().validate()
    }

    /// Every universe defined by a ``u=`` in the model, sorted ascending and
    /// deduplicated. Universe 0 (the real world) is not reported.
    fn universe_ids(&mut self) -> Vec<i64> {
        self.inner.view().universe_ids()
    }

    /// Carve universe ``u`` into a new :class:`Model`: its cells plus everything
    /// they reference -- surfaces, the cells reached through ``#n`` complements
    /// and ``LIKE n`` bases (followed transitively), and the materials and
    /// transforms those use. Only the data cards those references need are
    /// carried; global cards (the source, physics, ...) pass through, so the
    /// result runs on its own. Call :meth:`clear_data_cards` for a geometry-only
    /// sub-model instead.
    fn extract_universe(&mut self, u: i64) -> Model {
        Model::build(self.inner.extract_universe(u))
    }

    /// Carve the level-0 shell (every cell with no ``u=``) plus everything it
    /// references into a new :class:`Model`, following the same rules as
    /// :meth:`extract_universe`.
    fn extract_level0(&mut self) -> Model {
        Model::build(self.inner.extract_level0())
    }

    /// Drop every data card, keeping the title, cells, and surfaces. The model is
    /// re-parsed from the reduced source (live handles into it become stale).
    fn clear_data_cards(&mut self) {
        self.inner.clear_data_cards();
        self.invalidate();
    }

    /// Merge other models into this one, appending their cells, surfaces, and
    /// data cards. Honors the disjoint-range convention: if any cell, surface,
    /// material, or transform id is defined by more than one of the merged
    /// models, **nothing changes** and :class:`MergeError` is raised listing
    /// every conflict. On success the model is re-parsed from the combined
    /// source (live handles into it become stale).
    fn merge(&mut self, others: Vec<PyRef<'_, Model>>) -> PyResult<()> {
        let refs: Vec<&core::Model> = others.iter().map(|m| &m.inner).collect();
        match self.inner.merge(&refs) {
            Ok(()) => {
                self.invalidate();
                Ok(())
            }
            Err(conflicts) => Err(merge_error(&conflicts)),
        }
    }
}

/// A number-mapping supplied from Python: either a dict or a callable.
///
/// A callable is memoised so it is invoked at most once per distinct id, which
/// keeps whole-geometry renumbering fast even though many tokens share ids.
enum Mapper<'py> {
    Dict(FxHashMap<i64, i64>),
    Call {
        func: Bound<'py, PyAny>,
        cache: FxHashMap<i64, i64>,
        err: Option<PyErr>,
    },
}

impl<'py> Mapper<'py> {
    fn build(mapping: Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(dict) = mapping.downcast::<PyDict>() {
            let mut m = FxHashMap::default();
            for (k, v) in dict.iter() {
                m.insert(k.extract::<i64>()?, v.extract::<i64>()?);
            }
            Ok(Mapper::Dict(m))
        } else {
            Ok(Mapper::Call {
                func: mapping,
                cache: FxHashMap::default(),
                err: None,
            })
        }
    }

    fn map(&mut self, id: i64) -> i64 {
        match self {
            Mapper::Dict(m) => m.get(&id).copied().unwrap_or(id),
            Mapper::Call { func, cache, err } => {
                if let Some(&v) = cache.get(&id) {
                    return v;
                }
                match func.call1((id,)).and_then(|r| r.extract::<i64>()) {
                    Ok(v) => {
                        cache.insert(id, v);
                        v
                    }
                    Err(e) => {
                        if err.is_none() {
                            *err = Some(e);
                        }
                        id
                    }
                }
            }
        }
    }

    /// Surface the first error raised by a callable mapping, if any.
    fn into_result(self) -> PyResult<()> {
        match self {
            Mapper::Call { err: Some(e), .. } => Err(e),
            _ => Ok(()),
        }
    }
}

/// Parse MCNP `text` into a :class:`Model`.
#[pyfunction]
fn parse(text: &str) -> Model {
    Model::new(text)
}

/// migjorn -- a fast, lossless MCNP parser (private compiled module; the public
/// API is re-exported by the `migjorn` Python package).
#[pymodule]
fn _migjorn(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<Model>()?;
    m.add_class::<Surface>()?;
    m.add_class::<Cell>()?;
    m.add_class::<Material>()?;
    m.add_class::<Transform>()?;
    m.add_class::<Fill>()?;
    m.add_class::<CellParam>()?;
    m.add_class::<DataCard>()?;
    m.add_class::<Diagnostic>()?;
    m.add("MergeError", m.py().get_type::<MergeError>())?;
    Ok(())
}
