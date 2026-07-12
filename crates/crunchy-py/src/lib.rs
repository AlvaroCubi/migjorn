//! Python bindings for the crunchy MCNP parser.
//!
//! The public surface mirrors `crunchy-core`'s `Model` facade with Pythonic
//! ergonomics: `crunchy.parse(text)` (or `crunchy.Model.from_file(path)`) returns
//! a `Model` exposing typed access, id lookups, lossless re-emission, and
//! whole-geometry renumbering.
//!
//! The typed objects (`Cell`, `Surface`, `Material`, `Transform`) are **live
//! handles**, not snapshots: each holds a reference to its `Model` plus a stable
//! card slot. Getters read the current card on demand and setters (e.g.
//! `cell.material = 124`) write straight through the lossless override engine, so
//! edits are visible immediately and the rest of the model stays byte-for-byte.

use std::cell::OnceCell;

use crunchy_core as core;
use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rustc_hash::FxHashMap;

/// Error raised when a handle's card no longer exists (it was deleted, or the
/// model was structurally changed out from under it).
fn stale_handle() -> PyErr {
    PyRuntimeError::new_err("stale handle: the referenced card no longer exists")
}

/// Map a core editing error (all of which describe unsupported structural edits)
/// to a Python `ValueError`.
fn edit_error(e: core::EditError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// A cell card (a live handle onto its model).
///
/// Attributes:
///     id (int): Cell number.
///     material (int | None): Material number (0 = void; None for ``LIKE n BUT``).
///         Assignable in place (subject to the void/non-void restriction).
///     density (float | None): Density (positive = atom, negative = mass).
///         Assignable in place for a non-void cell.
///     is_void (bool): True when the material number is 0.
///     like (int | None): Base cell for a ``LIKE n BUT`` card.
///     surface_ids (list[int]): Referenced surface numbers (magnitudes).
///     signed_surfaces (list[int]): Referenced surfaces with sense (sign).
///     cell_refs (list[int]): Referenced cells (``#n`` complements, ``LIKE n``).
///     well_formed (bool): False if the geometry could not be fully parsed.
///     text (str): The card's exact source (incl. inline ``$`` comments),
///         reflecting any edits.
#[pyclass(module = "crunchy")]
struct Cell {
    model: Py<Model>,
    slot: u32,
}

impl Cell {
    /// Resolve this handle's current card and read it (preferring an edited
    /// view), running `f` on it. Raises if the card no longer exists or is no
    /// longer a cell.
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::CellRead) -> R) -> PyResult<R> {
        let m = self.model.bind(py).borrow();
        let ci = m
            .inner
            .tree()
            .card_by_slot(self.slot)
            .ok_or_else(stale_handle)?;
        let view = m
            .inner
            .read_cell(ci)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid cell"))?;
        Ok(f(&view))
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
            .tree()
            .card_by_slot(self.slot)
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
        self.read(py, |c| c.id())
    }
    #[getter]
    fn material(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.read(py, |c| c.material())
    }
    #[setter]
    fn set_material(&self, py: Python<'_>, value: i64) -> PyResult<()> {
        self.edit(py, |m, ci| m.set_cell_material(ci, value))
    }
    #[getter]
    fn density(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        self.read(py, |c| c.density())
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
        self.read(py, |c| c.like())
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
        self.read(py, |c| c.cell_refs())
    }
    #[getter]
    fn well_formed(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |c| c.well_formed())
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        Ok(tree.card_source(ci))
    }

    /// Intersect the geometry with a signed surface (negative int = negative
    /// sense). Restructures the card in place.
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

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |c| {
            format!(
                "Cell(id={}, material={:?}, density={:?})",
                c.id(),
                c.material(),
                c.density()
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
#[pyclass(module = "crunchy")]
struct Surface {
    model: Py<Model>,
    slot: u32,
}

impl Surface {
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Surface) -> R) -> PyResult<R> {
        let m = self.model.bind(py).borrow();
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        let surface = core::parse_surface(tree, ci)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid surface"))?;
        Ok(f(&surface))
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
    #[getter]
    fn transform(&self, py: Python<'_>) -> PyResult<Option<i64>> {
        self.read(py, |s| s.transform)
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
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        Ok(tree.card_source(ci))
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
#[pyclass(module = "crunchy")]
struct Material {
    model: Py<Model>,
    slot: u32,
}

impl Material {
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Material) -> R) -> PyResult<R> {
        let m = self.model.bind(py).borrow();
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        let material = core::parse_material(tree, ci)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid material"))?;
        Ok(f(&material))
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
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        Ok(tree.card_source(ci))
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
#[pyclass(module = "crunchy")]
struct Transform {
    model: Py<Model>,
    slot: u32,
}

impl Transform {
    fn read<R>(&self, py: Python<'_>, f: impl FnOnce(&core::Transform) -> R) -> PyResult<R> {
        let m = self.model.bind(py).borrow();
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        let transform = core::parse_transform(tree, ci)
            .ok_or_else(|| PyRuntimeError::new_err("card is no longer a valid transform"))?;
        Ok(f(&transform))
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
    #[getter]
    fn rotation(&self, py: Python<'_>) -> PyResult<Vec<f64>> {
        self.read(py, |t| t.rotation.clone())
    }
    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<String> {
        let m = self.model.bind(py).borrow();
        let tree = m.inner.tree();
        let ci = tree.card_by_slot(self.slot).ok_or_else(stale_handle)?;
        Ok(tree.card_source(ci))
    }
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.read(py, |t| {
            format!("Transform(id={}, degrees={})", t.id, t.degrees)
        })
    }
}

/// A generically-parsed data card (anything not modelled more specifically).
///
/// Attributes:
///     name (str): Uppercased mnemonic including any number (``"SDEF"``, ``"F4"``).
///     particle (str | None): Particle designator after ``:`` (``"n"``, ``"n,p"``).
///     starred (bool): Leading ``*`` modifier (``*F``, ``*TR``).
#[pyclass(frozen, module = "crunchy")]
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
#[pyclass(frozen, module = "crunchy")]
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
/// Construct with :func:`crunchy.parse`, ``Model(text)``, or
/// :meth:`Model.from_file`. The model is *lossless*: :meth:`to_source` (and
/// ``str(model)``) reproduce the input byte-for-byte until you edit it, and edits
/// such as :meth:`renumber_surfaces` or ``cell.material = ...`` change only the
/// affected tokens.
///
/// The ``cells``/``surfaces``/``materials``/``transforms``/``data_cards``
/// properties materialise handle lists -- convenient, but for very large models
/// prefer the id lookups (:meth:`surface`, :meth:`cell`, ...) and the ``num_*``
/// counts, which do not build the whole list.
#[pyclass(unsendable, module = "crunchy")]
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

    fn idx(&self) -> &core::ModelIndex {
        self.index.get_or_init(|| self.inner.index())
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

    fn __repr__(&self) -> String {
        let idx = self.idx();
        format!(
            "Model(cells={}, surfaces={}, materials={}, transforms={}, diagnostics={})",
            idx.cells.len(),
            idx.surfaces.len(),
            idx.materials.len(),
            idx.transforms.len(),
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
    #[getter]
    fn cells(slf: Bound<'_, Self>) -> Vec<Cell> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        let tree = m.inner.tree();
        (0..tree.cards().len())
            .filter(|&pos| core::parse_cell(tree, pos).is_some())
            .map(|pos| Cell {
                model: model_py.clone_ref(py),
                slot: tree.card_slot(pos),
            })
            .collect()
    }

    /// All surfaces, in source order (materialises a list of handles).
    #[getter]
    fn surfaces(slf: Bound<'_, Self>) -> Vec<Surface> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        let tree = m.inner.tree();
        (0..tree.cards().len())
            .filter(|&pos| core::parse_surface(tree, pos).is_some())
            .map(|pos| Surface {
                model: model_py.clone_ref(py),
                slot: tree.card_slot(pos),
            })
            .collect()
    }

    /// All ``TRn`` transforms, in source order.
    #[getter]
    fn transforms(slf: Bound<'_, Self>) -> Vec<Transform> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        let tree = m.inner.tree();
        (0..tree.cards().len())
            .filter(|&pos| core::parse_transform(tree, pos).is_some())
            .map(|pos| Transform {
                model: model_py.clone_ref(py),
                slot: tree.card_slot(pos),
            })
            .collect()
    }

    /// All ``Mn`` materials, in source order.
    #[getter]
    fn materials(slf: Bound<'_, Self>) -> Vec<Material> {
        let py = slf.py();
        let model_py: Py<Model> = slf.clone().unbind();
        let m = slf.borrow();
        let tree = m.inner.tree();
        (0..tree.cards().len())
            .filter(|&pos| core::parse_material(tree, pos).is_some())
            .map(|pos| Material {
                model: model_py.clone_ref(py),
                slot: tree.card_slot(pos),
            })
            .collect()
    }

    /// All data cards (generic view), in source order.
    #[getter]
    fn data_cards(&self) -> Vec<DataCard> {
        self.inner.data_cards().map(DataCard::from).collect()
    }

    /// Number of cells (cheap; does not build the cell list).
    #[getter]
    fn num_cells(&self) -> usize {
        self.idx().cells.len()
    }

    /// Number of surfaces (cheap; does not build the surface list).
    #[getter]
    fn num_surfaces(&self) -> usize {
        self.idx().surfaces.len()
    }

    /// Number of materials (cheap; does not build the material list).
    #[getter]
    fn num_materials(&self) -> usize {
        self.idx().materials.len()
    }

    /// Number of transforms (cheap; does not build the transform list).
    #[getter]
    fn num_transforms(&self) -> usize {
        self.idx().transforms.len()
    }

    /// Look up a surface by number, or ``None``.
    fn surface(slf: Bound<'_, Self>, id: i64) -> Option<Surface> {
        let m = slf.borrow();
        let ci = *m.idx().surfaces.get(&id)?;
        let slot = m.inner.tree().card_slot(ci);
        Some(Surface {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a cell by number, or ``None``.
    fn cell(slf: Bound<'_, Self>, id: i64) -> Option<Cell> {
        let m = slf.borrow();
        let ci = *m.idx().cells.get(&id)?;
        let slot = m.inner.tree().card_slot(ci);
        Some(Cell {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a material by number, or ``None``.
    fn material(slf: Bound<'_, Self>, id: i64) -> Option<Material> {
        let m = slf.borrow();
        let ci = *m.idx().materials.get(&id)?;
        let slot = m.inner.tree().card_slot(ci);
        Some(Material {
            model: slf.clone().unbind(),
            slot,
        })
    }

    /// Look up a transform by number, or ``None``.
    fn transform(slf: Bound<'_, Self>, id: i64) -> Option<Transform> {
        let m = slf.borrow();
        let ci = *m.idx().transforms.get(&id)?;
        let slot = m.inner.tree().card_slot(ci);
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

/// crunchy -- a fast, lossless MCNP parser (private compiled module; the public
/// API is re-exported by the `crunchy` Python package).
#[pymodule]
fn _crunchy(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<Model>()?;
    m.add_class::<Surface>()?;
    m.add_class::<Cell>()?;
    m.add_class::<Material>()?;
    m.add_class::<Transform>()?;
    m.add_class::<DataCard>()?;
    m.add_class::<Diagnostic>()?;
    Ok(())
}
