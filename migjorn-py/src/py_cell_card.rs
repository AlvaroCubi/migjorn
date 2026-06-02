use pyo3::prelude::*;

use migjorn::{Card, CellParam, GeoElement};

use crate::py_model::PyModel;

mod py_cell_param;
pub use py_cell_param::PyCellParameter;

#[pyclass(name = "CellCard")]
pub struct PyCell {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PyCell {
    #[getter]
    fn card_id(&self, py: Python<'_>) -> u32 {
        self.model.borrow(py).inner.cells[self.index].cell_id()
    }

    #[setter]
    fn set_card_id(&self, py: Python<'_>, new_id: u32) {
        self.model.borrow_mut(py).inner.cells[self.index].set_cell_id(new_id);
    }

    #[getter]
    fn material_id(&self, py: Python<'_>) -> u32 {
        self.model.borrow(py).inner.cells[self.index].material_id()
    }

    #[setter]
    fn set_material_id(&self, py: Python<'_>, v: u32) {
        if v == 0 {
            self.void_cell(py);
            return;
        }
        self.model.borrow_mut(py).inner.cells[self.index].set_material_id(v);
    }

    #[getter]
    fn density(&self, py: Python<'_>) -> Option<f64> {
        self.model.borrow(py).inner.cells[self.index].density()
    }

    #[setter]
    fn set_density(&self, py: Python<'_>, v: f64) {
        let mut m = self.model.borrow_mut(py);
        let cell = &mut m.inner.cells[self.index];
        cell.set_density(v);
    }

    /// Geometry as a read-only list of strings (surface IDs, operator chars).
    #[getter]
    fn geometry(&self, py: Python<'_>) -> Vec<String> {
        self.model.borrow(py).inner.cells[self.index]
            .geometry()
            .map(|elem| match elem {
                GeoElement::Surface(s) => s.to_string(),
                GeoElement::Cell(c) => c.to_string(),
                GeoElement::Operator(op) => op.to_string(),
            })
            .collect()
    }

    /// The cell parameters (IMP:N, VOL, FILL, etc.) as a list of
    /// :class:`CellParameter` objects. Iterate over them to inspect or modify.
    #[getter]
    fn params(&self, py: Python<'_>) -> Vec<Py<PyCellParameter>> {
        let count = self.model.borrow(py).inner.cells[self.index].params().len();
        (0..count)
            .map(|param_index| {
                Py::new(
                    py,
                    PyCellParameter {
                        model: self.model.clone_ref(py),
                        cell_index: self.index,
                        param_index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.cells[self.index]
            .original_text()
            .to_string()
    }

    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.cells[self.index].updated_text()
    }

    /// Remove the parameter at the given index.
    fn remove_param(&self, py: Python<'_>, index: usize) {
        self.model.borrow_mut(py).inner.cells[self.index].remove_param(index);
    }

    /// Add a new parameter of the given type with the given value at a given index.
    #[pyo3(signature = (text, index=None))]
    pub fn insert_parameter(
        &self,
        py: Python<'_>,
        text: &str,
        index: Option<usize>,
    ) -> PyResult<()> {
        let mut m = self.model.borrow_mut(py);
        let cell = &mut m.inner.cells[self.index];
        let target_index = index.unwrap_or(cell.params().len());
        let param = CellParam::try_from(text)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        cell.insert_param(target_index, param);
        Ok(())
    }

    fn void_cell(&self, py: Python<'_>) {
        self.model.borrow_mut(py).inner.cells[self.index].make_void();
    }

    fn remove_geometry_element(&self, py: Python<'_>, index: usize) {
        self.model.borrow_mut(py).inner.cells[self.index].remove_geometry_element(index);
    }

    #[pyo3(signature = (text, index=None))]
    fn insert_geometry_elements(
        &self,
        py: Python<'_>,
        text: &str,
        index: Option<usize>,
    ) -> PyResult<()> {
        let elements = GeoElement::parse_many(text)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let mut m = self.model.borrow_mut(py);
        let cell = &mut m.inner.cells[self.index];
        let base = index.unwrap_or(cell.geometry().collect::<Vec<_>>().len());
        for (i, elem) in elements.into_iter().enumerate() {
            cell.insert_geometry_element(base + i, elem);
        }
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        let cell = &m.inner.cells[self.index];
        format!(
            "<CellCard id={} mat={}>",
            cell.cell_id(),
            cell.material_id()
        )
    }
}
