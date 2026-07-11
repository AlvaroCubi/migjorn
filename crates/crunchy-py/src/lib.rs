//! Python bindings for the crunchy MCNP parser.
//!
//! The public surface mirrors `crunchy-core`'s `Deck` facade with Pythonic
//! ergonomics: `crunchy.parse(text)` (or `crunchy.Deck.from_file(path)`) returns
//! a `Deck` exposing typed lists, id lookups, lossless re-emission, and
//! whole-geometry renumbering.

use std::cell::OnceCell;

use crunchy_core as core;
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rustc_hash::FxHashMap;

/// A surface card.
///
/// Attributes:
///     id (int): Surface number.
///     kind (str): MCNP mnemonic, e.g. ``"PX"``, ``"GQ"``, ``"C/X"``, ``"RPP"``.
///     coeffs (list[float]): Surface coefficients.
///     transform (int | None): Transformation number (negative => periodic).
///     reflective (bool): Reflective boundary (leading ``*``).
///     white (bool): White boundary (leading ``+``).
///     well_formed (bool): False if a coefficient could not be parsed.
#[pyclass(frozen, module = "crunchy")]
#[derive(Clone)]
struct Surface {
    #[pyo3(get)]
    id: i64,
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    coeffs: Vec<f64>,
    #[pyo3(get)]
    transform: Option<i64>,
    #[pyo3(get)]
    reflective: bool,
    #[pyo3(get)]
    white: bool,
    #[pyo3(get)]
    well_formed: bool,
}

impl From<core::Surface> for Surface {
    fn from(s: core::Surface) -> Self {
        Surface {
            id: s.id,
            kind: s.kind.mnemonic(),
            coeffs: s.coeffs,
            transform: s.transform,
            reflective: s.reflective,
            white: s.white,
            well_formed: s.well_formed,
        }
    }
}

#[pymethods]
impl Surface {
    fn __repr__(&self) -> String {
        format!(
            "Surface(id={}, kind={:?}, coeffs={:?})",
            self.id, self.kind, self.coeffs
        )
    }
}

/// A cell card.
///
/// Attributes:
///     id (int): Cell number.
///     material (int | None): Material number (0 = void; None for ``LIKE n BUT``).
///     density (float | None): Density (positive = atom, negative = mass).
///     is_void (bool): True when the material number is 0.
///     like (int | None): Base cell for a ``LIKE n BUT`` card.
///     surface_ids (list[int]): Referenced surface numbers (magnitudes).
///     signed_surfaces (list[int]): Referenced surfaces with sense (sign).
///     cell_refs (list[int]): Referenced cells (``#n`` complements, ``LIKE n``).
///     well_formed (bool): False if the geometry could not be fully parsed.
#[pyclass(frozen, module = "crunchy")]
#[derive(Clone)]
struct Cell {
    #[pyo3(get)]
    id: i64,
    #[pyo3(get)]
    material: Option<i64>,
    #[pyo3(get)]
    density: Option<f64>,
    #[pyo3(get)]
    is_void: bool,
    #[pyo3(get)]
    like: Option<i64>,
    #[pyo3(get)]
    surface_ids: Vec<i64>,
    #[pyo3(get)]
    signed_surfaces: Vec<i64>,
    #[pyo3(get)]
    cell_refs: Vec<i64>,
    #[pyo3(get)]
    well_formed: bool,
}

impl From<core::Cell> for Cell {
    fn from(c: core::Cell) -> Self {
        let surface_refs = c.surface_refs();
        let surface_ids = surface_refs.iter().map(|r| r.id).collect();
        let signed_surfaces = surface_refs
            .iter()
            .map(|r| if r.negative { -r.id } else { r.id })
            .collect();
        let cell_refs = c.cell_refs().iter().map(|r| r.id).collect();
        Cell {
            id: c.id,
            material: c.material,
            density: c.density,
            is_void: c.material == Some(0),
            like: c.like.map(|r| r.id),
            surface_ids,
            signed_surfaces,
            cell_refs,
            well_formed: c.well_formed,
        }
    }
}

#[pymethods]
impl Cell {
    fn __repr__(&self) -> String {
        format!(
            "Cell(id={}, material={:?}, density={:?}, surfaces={:?})",
            self.id, self.material, self.density, self.signed_surfaces
        )
    }
}

/// A material (``Mn``) card.
///
/// Attributes:
///     id (int): Material number.
///     entries (list[tuple[str, float]]): ``(zaid, fraction)`` pairs; a
///         positive fraction is atomic, a negative fraction is by weight.
///     well_formed (bool): False if entries were not clean ZAID/fraction pairs.
#[pyclass(frozen, module = "crunchy")]
#[derive(Clone)]
struct Material {
    #[pyo3(get)]
    id: i64,
    #[pyo3(get)]
    entries: Vec<(String, f64)>,
    #[pyo3(get)]
    well_formed: bool,
}

impl From<core::Material> for Material {
    fn from(m: core::Material) -> Self {
        Material {
            id: m.id,
            entries: m
                .entries
                .into_iter()
                .map(|e| (e.zaid, e.fraction))
                .collect(),
            well_formed: m.well_formed,
        }
    }
}

#[pymethods]
impl Material {
    fn __repr__(&self) -> String {
        format!("Material(id={}, entries={})", self.id, self.entries.len())
    }
}

/// A coordinate transformation (``TRn`` / ``*TRn``) card.
///
/// Attributes:
///     id (int): Transformation number.
///     degrees (bool): Rotation entries are angles in degrees (``*TRn``).
///     displacement (tuple[float, float, float]): Origin displacement.
///     rotation (list[float]): Rotation entries as written.
#[pyclass(frozen, module = "crunchy")]
#[derive(Clone)]
struct Transform {
    #[pyo3(get)]
    id: i64,
    #[pyo3(get)]
    degrees: bool,
    #[pyo3(get)]
    displacement: (f64, f64, f64),
    #[pyo3(get)]
    rotation: Vec<f64>,
}

impl From<core::Transform> for Transform {
    fn from(t: core::Transform) -> Self {
        Transform {
            id: t.id,
            degrees: t.degrees,
            displacement: (t.displacement[0], t.displacement[1], t.displacement[2]),
            rotation: t.rotation,
        }
    }
}

#[pymethods]
impl Transform {
    fn __repr__(&self) -> String {
        format!("Transform(id={}, degrees={})", self.id, self.degrees)
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

/// A parsed MCNP deck.
///
/// Construct with :func:`crunchy.parse`, ``Deck(text)``, or
/// :meth:`Deck.from_file`. The deck is *lossless*: :meth:`to_source` (and
/// ``str(deck)``) reproduce the input byte-for-byte until you edit it, and edits
/// such as :meth:`renumber_surfaces` change only the affected numbers.
///
/// The ``cells``/``surfaces``/``materials``/``transforms``/``data_cards``
/// properties materialise full lists -- convenient, but for very large decks
/// prefer the id lookups (:meth:`surface`, :meth:`cell`, ...) and the ``num_*``
/// counts, which do not build the whole list.
#[pyclass(unsendable, module = "crunchy")]
struct Deck {
    inner: core::Deck,
    index: OnceCell<core::DeckIndex>,
}

impl Deck {
    fn build(inner: core::Deck) -> Self {
        Deck {
            inner,
            index: OnceCell::new(),
        }
    }

    fn idx(&self) -> &core::DeckIndex {
        self.index.get_or_init(|| self.inner.index())
    }

    /// Invalidate the cached id index after a structural/number edit.
    fn invalidate(&mut self) {
        self.index = OnceCell::new();
    }
}

#[pymethods]
impl Deck {
    /// Parse MCNP `text` into a deck.
    #[new]
    fn new(text: &str) -> Self {
        Deck::build(core::Deck::parse(text))
    }

    /// Parse the MCNP deck at `path`.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| PyIOError::new_err(format!("cannot read {path}: {e}")))?;
        Ok(Deck::build(core::Deck::parse(text)))
    }

    /// Re-emit the deck as MCNP text (byte-for-byte identical to the input when
    /// unedited; only edited numbers differ otherwise).
    fn to_source(&self) -> String {
        self.inner.to_source()
    }

    /// Write the deck to `path`.
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
            "Deck(cells={}, surfaces={}, materials={}, transforms={}, diagnostics={})",
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
        self.inner.diagnostics().iter().map(Diagnostic::from).collect()
    }

    /// All cells, in source order (materialises a list).
    #[getter]
    fn cells(&self) -> Vec<Cell> {
        self.inner.cells().map(Cell::from).collect()
    }

    /// All surfaces, in source order (materialises a list).
    #[getter]
    fn surfaces(&self) -> Vec<Surface> {
        self.inner.surfaces().map(Surface::from).collect()
    }

    /// All ``TRn`` transforms, in source order.
    #[getter]
    fn transforms(&self) -> Vec<Transform> {
        self.inner.transforms().map(Transform::from).collect()
    }

    /// All ``Mn`` materials, in source order.
    #[getter]
    fn materials(&self) -> Vec<Material> {
        self.inner.materials().map(Material::from).collect()
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

    /// Look up a surface by number, or ``None``.
    fn surface(&self, id: i64) -> Option<Surface> {
        let ci = *self.idx().surfaces.get(&id)?;
        core::parse_surface(self.inner.tree(), ci).map(Surface::from)
    }

    /// Look up a cell by number, or ``None``.
    fn cell(&self, id: i64) -> Option<Cell> {
        let ci = *self.idx().cells.get(&id)?;
        core::parse_cell(self.inner.tree(), ci).map(Cell::from)
    }

    /// Look up a material by number, or ``None``.
    fn material(&self, id: i64) -> Option<Material> {
        let ci = *self.idx().materials.get(&id)?;
        core::parse_material(self.inner.tree(), ci).map(Material::from)
    }

    /// Look up a transform by number, or ``None``.
    fn transform(&self, id: i64) -> Option<Transform> {
        let ci = *self.idx().transforms.get(&id)?;
        core::parse_transform(self.inner.tree(), ci).map(Transform::from)
    }

    /// Renumber every surface -- definitions **and** all references in cell
    /// geometry -- consistently.
    ///
    /// `mapping` may be either:
    ///   * a ``dict[int, int]`` (old number -> new number; unmapped numbers are
    ///     left unchanged), or
    ///   * a callable ``int -> int`` (invoked once per distinct surface number).
    ///
    /// Example::
    ///
    ///     deck.renumber_surfaces(lambda n: n + 1000)
    ///     deck.renumber_surfaces({1: 100, 2: 200})
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

/// Parse MCNP `text` into a :class:`Deck`.
#[pyfunction]
fn parse(text: &str) -> Deck {
    Deck::new(text)
}

/// crunchy -- a fast, lossless MCNP parser.
#[pymodule]
fn crunchy(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<Deck>()?;
    m.add_class::<Surface>()?;
    m.add_class::<Cell>()?;
    m.add_class::<Material>()?;
    m.add_class::<Transform>()?;
    m.add_class::<DataCard>()?;
    m.add_class::<Diagnostic>()?;
    Ok(())
}
