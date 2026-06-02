use pyo3::prelude::*;

use migjorn::Card;

use crate::py_model::PyModel;

#[pyclass(name = "SurfaceCard")]
pub struct PySurface {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PySurface {
    /// The card ID (surface number).
    #[getter]
    fn card_id(&self, py: Python<'_>) -> u32 {
        self.model.borrow(py).inner.surfaces[self.index].surface_id()
    }

    /// Set the card ID (surface number).
    #[setter]
    fn set_card_id(&self, py: Python<'_>, new_id: u32) {
        self.model.borrow_mut(py).inner.surfaces[self.index].set_surface_id(new_id);
    }

    /// Read-only: the surface writer only splices the surface ID, not coefficients.
    #[getter]
    fn coeffs(&self, py: Python<'_>) -> Vec<f64> {
        self.model.borrow(py).inner.surfaces[self.index]
            .coeffs()
            .to_vec()
    }

    #[getter]
    fn transform_id(&self, py: Python<'_>) -> Option<u32> {
        self.model.borrow(py).inner.surfaces[self.index].transform_id()
    }

    #[getter]
    fn surface_type(&self, py: Python<'_>) -> String {
        format!(
            "{:?}",
            self.model.borrow(py).inner.surfaces[self.index].surface_type()
        )
    }

    /// The original card text as it appears in the source file.
    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.surfaces[self.index]
            .original_text()
            .to_string()
    }

    /// The card text with all in-memory modifications applied.
    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.surfaces[self.index].updated_text()
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        let s = &m.inner.surfaces[self.index];
        format!(
            "<SurfaceCard id={} type={:?}>",
            s.surface_id(),
            s.surface_type()
        )
    }
}
