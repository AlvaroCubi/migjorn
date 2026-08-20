//! PyO3 bindings (milestone M6): the `migjorn` Python extension.
//!
//! Handles (`Cell`, `Surface`, ...) are live and cheap — each is a shared
//! reference to the one `Model` plus the stable `slot` of its card, exactly the
//! Rust design. `model.cell(7)` always resolves against current state; a handle
//! to a card you `remove()` raises on use. Nothing here reparses the file.
//!
//! The model is shared through `Rc<RefCell<..>>`: every access happens under the
//! GIL, so a single-threaded interior-mutability cell is enough and avoids lock
//! overhead. The class is therefore `unsendable`.

use std::cell::RefCell;
use std::rc::Rc;

use pyo3::create_exception;
use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use migjorn_core::EditError;

type Shared = Rc<RefCell<migjorn_core::Model>>;

create_exception!(
    migjorn,
    MergeError,
    PyValueError,
    "Raised by Model.merge on a shared cell/surface/material/transform id."
);

fn removed() -> PyErr {
    PyValueError::new_err("handle refers to a card that has been removed")
}

fn edit_err(e: EditError) -> PyErr {
    let msg = match e {
        EditError::WrongKind => "edit does not apply to this card",
        EditError::NoSuchField => "the addressed field does not exist on this card",
        EditError::NoBlock => "no block of that kind to add the card into",
    };
    PyValueError::new_err(msg)
}

/// Render an `Option<i64>` the way a Python repr would: the int, or `None`.
fn py_opt_i64(v: Option<i64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "None".to_owned(),
    }
}

// ===========================================================================
// Model
// ===========================================================================

/// A parsed MCNP model: lossless, typed access, cheap iterative editing.
///
/// `parse(t).to_source() == t` byte-for-byte, and stays lossless except at cards
/// you edit. No operation re-parses the whole file, and reads always reflect the
/// latest edit with no explicit flush.
#[pyclass(unsendable)]
struct Model {
    inner: Shared,
}

#[pymethods]
impl Model {
    #[new]
    fn new(text: &str) -> Self {
        Model {
            inner: Rc::new(RefCell::new(migjorn_core::Model::parse(text))),
        }
    }

    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Model> {
        let text = std::fs::read_to_string(path).map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(Model::new(&text))
    }

    fn to_source(&self) -> String {
        self.inner.borrow().to_source()
    }

    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.inner.borrow().to_source())
            .map_err(|e| PyIOError::new_err(e.to_string()))
    }

    #[getter]
    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.inner
            .borrow()
            .diagnostics()
            .iter()
            .map(|d| Diagnostic {
                severity: match d.severity {
                    migjorn_core::Severity::Error => "error",
                    migjorn_core::Severity::Warning => "warning",
                }
                .to_owned(),
                message: d.message.clone(),
                start: d.span.start,
                end: d.span.end,
            })
            .collect()
    }

    #[getter]
    fn title(&self) -> Option<String> {
        self.inner.borrow().title().map(str::to_owned)
    }
    #[setter]
    fn set_title(&self, value: &str) {
        self.inner.borrow_mut().set_title(value);
    }

    #[getter]
    fn num_cells(&self) -> usize {
        self.inner.borrow().num_cells()
    }
    #[getter]
    fn num_surfaces(&self) -> usize {
        self.inner.borrow().num_surfaces()
    }
    #[getter]
    fn num_materials(&self) -> usize {
        self.inner.borrow().num_materials()
    }
    #[getter]
    fn num_transforms(&self) -> usize {
        self.inner.borrow().num_transforms()
    }

    fn cell(&self, id: i64) -> Option<Cell> {
        let slot = self.inner.borrow().cell(id)?.slot();
        Some(Cell {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn surface(&self, id: i64) -> Option<Surface> {
        let slot = self.inner.borrow().surface(id)?.slot();
        Some(Surface {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn material(&self, id: i64) -> Option<Material> {
        let slot = self.inner.borrow().material(id)?.slot();
        Some(Material {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn transform(&self, id: i64) -> Option<Transform> {
        let slot = self.inner.borrow().transform(id)?.slot();
        Some(Transform {
            inner: self.inner.clone(),
            slot,
        })
    }

    fn cells(&self) -> Vec<Cell> {
        let m = self.inner.borrow();
        m.cells()
            .map(|v| Cell {
                inner: self.inner.clone(),
                slot: v.slot(),
            })
            .collect()
    }
    fn surfaces(&self) -> Vec<Surface> {
        let m = self.inner.borrow();
        m.surfaces()
            .map(|v| Surface {
                inner: self.inner.clone(),
                slot: v.slot(),
            })
            .collect()
    }
    fn materials(&self) -> Vec<Material> {
        let m = self.inner.borrow();
        m.materials()
            .map(|v| Material {
                inner: self.inner.clone(),
                slot: v.slot(),
            })
            .collect()
    }
    fn transforms(&self) -> Vec<Transform> {
        let m = self.inner.borrow();
        m.transforms()
            .map(|v| Transform {
                inner: self.inner.clone(),
                slot: v.slot(),
            })
            .collect()
    }
    fn data_cards(&self) -> Vec<DataCard> {
        let m = self.inner.borrow();
        m.data_cards()
            .map(|v| DataCard {
                inner: self.inner.clone(),
                slot: v.slot(),
            })
            .collect()
    }

    // --- structural edits ---------------------------------------------------
    fn add_cell(&self, text: &str) -> PyResult<Cell> {
        let slot = self.inner.borrow_mut().add_cell(text).map_err(edit_err)?;
        Ok(Cell {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn add_surface(&self, text: &str) -> PyResult<Surface> {
        let slot = self
            .inner
            .borrow_mut()
            .add_surface(text)
            .map_err(edit_err)?;
        Ok(Surface {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn add_material(&self, text: &str) -> PyResult<Material> {
        let slot = self
            .inner
            .borrow_mut()
            .add_material(text)
            .map_err(edit_err)?;
        Ok(Material {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn add_transform(&self, text: &str) -> PyResult<Transform> {
        let slot = self
            .inner
            .borrow_mut()
            .add_transform(text)
            .map_err(edit_err)?;
        Ok(Transform {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn add_data_card(&self, text: &str) -> PyResult<DataCard> {
        let slot = self
            .inner
            .borrow_mut()
            .add_data_card(text)
            .map_err(edit_err)?;
        Ok(DataCard {
            inner: self.inner.clone(),
            slot,
        })
    }
    fn remove_cell(&self, id: i64) -> bool {
        self.inner.borrow_mut().remove_cell(id)
    }
    fn remove_surface(&self, id: i64) -> bool {
        self.inner.borrow_mut().remove_surface(id)
    }
    fn remove_material(&self, id: i64) -> bool {
        self.inner.borrow_mut().remove_material(id)
    }
    fn remove_transform(&self, id: i64) -> bool {
        self.inner.borrow_mut().remove_transform(id)
    }

    // --- renumbering --------------------------------------------------------
    fn renumber_cells(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Cells)
    }
    fn renumber_surfaces(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Surfaces)
    }
    fn renumber_materials(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Materials)
    }
    fn renumber_transforms(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Transforms)
    }
    fn renumber_universes(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Universes)
    }
    fn renumber_tallies(&self, mapping: &Bound<'_, PyDict>) -> PyResult<()> {
        do_renumber(&self.inner, mapping, RKind::Tallies)
    }
    fn offset_cells(&self, delta: i64) {
        self.inner.borrow_mut().offset_cells(delta);
    }
    fn offset_surfaces(&self, delta: i64) {
        self.inner.borrow_mut().offset_surfaces(delta);
    }

    // --- analysis / composition ---------------------------------------------
    fn validate(&self) -> Vec<String> {
        self.inner.borrow().validate()
    }
    fn universe_ids(&self) -> Vec<i64> {
        self.inner.borrow().universe_ids()
    }
    fn extract_universe(&self, u: i64) -> Model {
        Model {
            inner: Rc::new(RefCell::new(self.inner.borrow().extract_universe(u))),
        }
    }
    fn extract_level0(&self) -> Model {
        Model {
            inner: Rc::new(RefCell::new(self.inner.borrow().extract_level0())),
        }
    }
    fn extract_cells(&self, ids: Vec<i64>) -> Model {
        Model {
            inner: Rc::new(RefCell::new(self.inner.borrow().extract_cells(&ids))),
        }
    }
    fn merge(&self, others: Vec<Bound<'_, Model>>) -> PyResult<()> {
        // Deref the `Ref` to the `Model` before cloning, so this is unambiguously
        // `Model::clone` (a bare `.clone()` on a `Ref` reads as `Ref::clone`).
        let owned: Vec<migjorn_core::Model> = others
            .iter()
            .map(|b| (*b.borrow().inner.borrow()).clone())
            .collect();
        self.inner
            .borrow_mut()
            .merge(owned)
            .map_err(|errs| MergeError::new_err(errs.join("; ")))
    }

    fn __repr__(&self) -> String {
        let m = self.inner.borrow();
        let title = match m.title() {
            Some(t) => format!("{t:?}"),
            None => "None".to_owned(),
        };
        format!(
            "Model(title={title}, {} cells, {} surfaces, {} materials, {} transforms)",
            m.num_cells(),
            m.num_surfaces(),
            m.num_materials(),
            m.num_transforms()
        )
    }

    fn __str__(&self) -> String {
        self.inner.borrow().to_source()
    }
}

// ===========================================================================
// renumber mapping adaptation (dict -> Fn(i64) -> i64 + Sync)
// ===========================================================================

enum RKind {
    Cells,
    Surfaces,
    Materials,
    Transforms,
    Universes,
    Tallies,
}

/// Snapshot a Python `dict[int, int]` into a Rust `HashMap` once (ids absent
/// from it map to themselves). The resulting closure only reads that table, so
/// it is `Fn + Sync` — safe to call concurrently from any thread with no GIL
/// involved, unlike a Python callable would have been.
fn build_map(mapping: &Bound<'_, PyDict>) -> PyResult<impl Fn(i64) -> i64 + Sync> {
    let mut table = std::collections::HashMap::new();
    for (k, v) in mapping.iter() {
        table.insert(k.extract::<i64>()?, v.extract::<i64>()?);
    }
    Ok(move |i| *table.get(&i).unwrap_or(&i))
}

fn do_renumber(inner: &Shared, mapping: &Bound<'_, PyDict>, kind: RKind) -> PyResult<()> {
    let f = build_map(mapping)?;
    let mut m = inner.borrow_mut();
    match kind {
        RKind::Cells => m.renumber_cells(f),
        RKind::Surfaces => m.renumber_surfaces(f),
        RKind::Materials => m.renumber_materials(f),
        RKind::Transforms => m.renumber_transforms(f),
        RKind::Universes => m.renumber_universes(f),
        RKind::Tallies => m.renumber_tallies(f),
    }
    Ok(())
}

// ===========================================================================
// Cell
// ===========================================================================

/// A live handle onto a cell card. Reads reflect current model state; a handle to
/// a removed cell raises `ValueError` on use.
#[pyclass(unsendable)]
struct Cell {
    inner: Shared,
    slot: u32,
}

#[pymethods]
impl Cell {
    #[getter]
    fn id(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .id())
    }
    #[getter]
    fn material(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .material())
    }
    #[setter]
    fn set_material(&self, value: i64) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_cell_material(self.slot, value)
            .map_err(edit_err)
    }
    #[getter]
    fn density(&self) -> PyResult<Option<f64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .density())
    }
    #[setter]
    fn set_density(&self, value: f64) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_cell_density(self.slot, value)
            .map_err(edit_err)
    }
    #[getter]
    fn is_void(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .is_void())
    }
    #[getter]
    fn like(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .like())
    }
    #[getter]
    fn universe(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .universe())
    }
    #[getter]
    fn signed_surfaces(&self) -> PyResult<Vec<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .signed_surfaces())
    }
    #[getter]
    fn surface_ids(&self) -> PyResult<Vec<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .surface_ids())
    }
    #[getter]
    fn cell_refs(&self) -> PyResult<Vec<i64>> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .cell_refs())
    }
    #[getter]
    fn well_formed(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .well_formed())
    }
    #[getter]
    fn text(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
    #[getter]
    fn fill(&self) -> PyResult<Option<Fill>> {
        let m = self.inner.borrow();
        let v = m.cell_at(self.slot).ok_or_else(removed)?;
        Ok(v.fill().map(|f| Fill {
            universe: f.universe,
            starred: f.starred,
            transform: f.transform,
        }))
    }
    #[getter]
    fn params(&self) -> PyResult<Vec<CellParam>> {
        let m = self.inner.borrow();
        let v = m.cell_at(self.slot).ok_or_else(removed)?;
        Ok(v.params().into_iter().map(cell_param).collect())
    }

    fn param(&self, key: &str) -> PyResult<Option<CellParam>> {
        let m = self.inner.borrow();
        let v = m.cell_at(self.slot).ok_or_else(removed)?;
        Ok(v.param(key).map(cell_param))
    }
    fn set_param(&self, key: &str, value: &str) -> PyResult<bool> {
        self.inner
            .borrow_mut()
            .set_cell_param(self.slot, key, value)
            .map_err(edit_err)
    }
    fn add_param(&self, text: &str) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .add_cell_param(self.slot, text)
            .map_err(edit_err)
    }
    fn remove_param(&self, key: &str) -> PyResult<bool> {
        self.inner
            .borrow_mut()
            .remove_cell_param(self.slot, key)
            .map_err(edit_err)
    }
    fn append_comment(&self, text: &str) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .append_cell_comment(self.slot, text)
            .map_err(edit_err)
    }

    fn __repr__(&self) -> String {
        match self.inner.borrow().cell_at(self.slot) {
            Some(v) => format!("Cell(id={})", py_opt_i64(v.id())),
            None => "Cell(<removed>)".to_owned(),
        }
    }
    fn __str__(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .cell_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
}

fn cell_param(p: migjorn_core::CellParam) -> CellParam {
    CellParam {
        key: p.key.clone(),
        particle: p.particle.clone(),
        starred: p.starred,
        value: p.value.clone(),
    }
}

// ===========================================================================
// Surface
// ===========================================================================

/// A live handle onto a surface card.
#[pyclass(unsendable)]
struct Surface {
    inner: Shared,
    slot: u32,
}

#[pymethods]
impl Surface {
    #[getter]
    fn id(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .id())
    }
    #[getter]
    fn kind(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .kind()
            .map(str::to_owned))
    }
    #[getter]
    fn coeffs(&self) -> PyResult<Vec<f64>> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .coeffs())
    }
    #[setter]
    fn set_coeffs(&self, values: Vec<f64>) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_surface_coeffs(self.slot, &values)
            .map_err(edit_err)
    }
    #[getter]
    fn transform(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .transform())
    }
    #[setter]
    fn set_transform(&self, value: Option<i64>) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_surface_transform(self.slot, value)
            .map_err(edit_err)
    }
    #[getter]
    fn reflective(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .reflective())
    }
    #[getter]
    fn white(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .white())
    }
    #[getter]
    fn well_formed(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .well_formed())
    }
    #[getter]
    fn text(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }

    fn set_coeff(&self, index: usize, value: f64) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_surface_coeff(self.slot, index, value)
            .map_err(edit_err)
    }

    fn __repr__(&self) -> String {
        match self.inner.borrow().surface_at(self.slot) {
            Some(v) => format!("Surface(id={})", py_opt_i64(v.id())),
            None => "Surface(<removed>)".to_owned(),
        }
    }
    fn __str__(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .surface_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
}

// ===========================================================================
// Material
// ===========================================================================

/// A live handle onto an `Mn` material card.
#[pyclass(unsendable)]
struct Material {
    inner: Shared,
    slot: u32,
}

#[pymethods]
impl Material {
    #[getter]
    fn id(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .material_at(self.slot)
            .ok_or_else(removed)?
            .id())
    }
    #[getter]
    fn entries(&self) -> PyResult<Vec<(String, f64)>> {
        Ok(self
            .inner
            .borrow()
            .material_at(self.slot)
            .ok_or_else(removed)?
            .entries())
    }
    #[getter]
    fn well_formed(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .material_at(self.slot)
            .ok_or_else(removed)?
            .well_formed())
    }
    #[getter]
    fn text(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .material_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }

    fn set_fraction(&self, entry: usize, value: f64) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_material_fraction(self.slot, entry, value)
            .map_err(edit_err)
    }
    fn set_zaid(&self, entry: usize, zaid: &str) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_material_zaid(self.slot, entry, zaid)
            .map_err(edit_err)
    }

    fn __repr__(&self) -> String {
        match self.inner.borrow().material_at(self.slot) {
            Some(v) => format!("Material(id={})", py_opt_i64(v.id())),
            None => "Material(<removed>)".to_owned(),
        }
    }
    fn __str__(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .material_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
}

// ===========================================================================
// Transform
// ===========================================================================

/// A live handle onto a `TRn` / `*TRn` transform card.
#[pyclass(unsendable)]
struct Transform {
    inner: Shared,
    slot: u32,
}

#[pymethods]
impl Transform {
    #[getter]
    fn id(&self) -> PyResult<Option<i64>> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .id())
    }
    #[getter]
    fn degrees(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .degrees())
    }
    #[getter]
    fn displacement(&self) -> PyResult<Vec<f64>> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .displacement())
    }
    #[getter]
    fn coeffs(&self) -> PyResult<Vec<f64>> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .coeffs())
    }
    #[setter]
    fn set_coeffs(&self, values: Vec<f64>) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_transform_coeffs(self.slot, &values)
            .map_err(edit_err)
    }
    #[getter]
    fn text(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }

    fn __repr__(&self) -> String {
        match self.inner.borrow().transform_at(self.slot) {
            Some(v) => format!("Transform(id={})", py_opt_i64(v.id())),
            None => "Transform(<removed>)".to_owned(),
        }
    }
    fn __str__(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .transform_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
}

// ===========================================================================
// DataCard
// ===========================================================================

/// A live handle onto any data card — the generic superset view. This
/// includes `Mn` and `TRn` cards too (they are `Data` cards like everything
/// else); prefer `Material` / `Transform` for those, since they have an id
/// to address by and get maintained in the model's id index. `DataCard` has
/// none, so it is addressed and removed by its own handle instead.
#[pyclass(unsendable)]
struct DataCard {
    inner: Shared,
    slot: u32,
}

#[pymethods]
impl DataCard {
    #[getter]
    fn text(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .data_card_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
    #[setter]
    fn set_text(&self, value: &str) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_data_card_text(self.slot, value)
            .map_err(edit_err)
    }
    #[getter]
    fn name(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .borrow()
            .data_card_at(self.slot)
            .ok_or_else(removed)?
            .name()
            .map(str::to_owned))
    }
    #[getter]
    fn particle(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .borrow()
            .data_card_at(self.slot)
            .ok_or_else(removed)?
            .particle())
    }
    #[getter]
    fn starred(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .borrow()
            .data_card_at(self.slot)
            .ok_or_else(removed)?
            .starred())
    }

    fn remove(&self) -> bool {
        self.inner.borrow_mut().remove_data_card(self.slot)
    }

    fn __repr__(&self) -> String {
        match self.inner.borrow().data_card_at(self.slot) {
            Some(v) => match v.name() {
                Some(name) => format!("DataCard(name={name:?})"),
                None => "DataCard(name=None)".to_owned(),
            },
            None => "DataCard(<removed>)".to_owned(),
        }
    }
    fn __str__(&self) -> PyResult<String> {
        Ok(self
            .inner
            .borrow()
            .data_card_at(self.slot)
            .ok_or_else(removed)?
            .text()
            .to_owned())
    }
}

// ===========================================================================
// Value objects
// ===========================================================================

/// A cell's single-universe `fill=` entry.
#[pyclass(unsendable)]
struct Fill {
    #[pyo3(get)]
    universe: i64,
    #[pyo3(get)]
    starred: bool,
    #[pyo3(get)]
    transform: Option<String>,
}

/// One `keyword[:particle][=]value` entry on a cell card.
#[pyclass(unsendable)]
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

/// A problem recorded while parsing; the offending bytes are still preserved.
#[pyclass(unsendable)]
struct Diagnostic {
    #[pyo3(get)]
    severity: String,
    #[pyo3(get)]
    message: String,
    #[pyo3(get)]
    start: usize,
    #[pyo3(get)]
    end: usize,
}

// ===========================================================================
// Module
// ===========================================================================

/// Parse MCNP input text into a `Model`. Never raises on malformed input —
/// problems are collected in `Model.diagnostics` and the bytes are preserved.
/// Equivalent to `Model(text)`.
#[pyfunction]
fn parse(text: &str) -> Model {
    Model::new(text)
}

#[pymodule]
fn migjorn(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<Model>()?;
    m.add_class::<Cell>()?;
    m.add_class::<Surface>()?;
    m.add_class::<Material>()?;
    m.add_class::<Transform>()?;
    m.add_class::<DataCard>()?;
    m.add_class::<Fill>()?;
    m.add_class::<CellParam>()?;
    m.add_class::<Diagnostic>()?;
    m.add("MergeError", m.py().get_type::<MergeError>())?;
    Ok(())
}
