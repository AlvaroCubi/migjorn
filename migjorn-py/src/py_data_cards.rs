use pyo3::prelude::*;

use migjorn::{Card, DataCard, Zaid};

use crate::py_model::PyModel;

// ── Zaid (standalone, no model reference needed) ──────────────────────────────

#[pyclass(name = "Zaid")]
pub struct PyZaid {
    isotope: u32,
    library: String,
    fraction: f64,
}

#[pymethods]
impl PyZaid {
    #[getter]
    fn isotope(&self) -> u32 {
        self.isotope
    }

    #[getter]
    fn library(&self) -> String {
        self.library.clone()
    }

    #[getter]
    fn fraction(&self) -> f64 {
        self.fraction
    }

    fn __repr__(&self) -> String {
        format!("<Zaid {}.{} {}>", self.isotope, self.library, self.fraction)
    }
}

pub fn zaid_to_py(py: Python<'_>, z: &Zaid) -> Py<PyZaid> {
    Py::new(
        py,
        PyZaid {
            isotope: z.isotope,
            library: z.library.clone(),
            fraction: z.fraction,
        },
    )
    .unwrap()
}

// ── Material card ─────────────────────────────────────────────────────────────

#[pyclass(name = "MaterialCard")]
pub struct PyMaterial {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PyMaterial {
    #[getter]
    fn card_id(&self, py: Python<'_>) -> u32 {
        self.material_id(py)
    }

    #[setter]
    fn set_card_id(&self, py: Python<'_>, new_id: u32) {
        self.set_material_id(py, new_id);
    }

    #[getter]
    fn material_id(&self, py: Python<'_>) -> u32 {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Material(mat) => mat.material_id(),
            _ => unreachable!(),
        }
    }

    #[setter]
    fn set_material_id(&self, py: Python<'_>, new_id: u32) {
        let mut m = self.model.borrow_mut(py);
        match &mut m.inner.data_cards[self.index] {
            DataCard::Material(mat) => mat.set_material_id(new_id),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn zaids(&self, py: Python<'_>) -> Vec<Py<PyZaid>> {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Material(mat) => mat.zaids().iter().map(|z| zaid_to_py(py, z)).collect(),
            _ => unreachable!(),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Material(mat) => format!(
                "<MaterialCard id={} zaids={}>",
                mat.material_id(),
                mat.zaids().len()
            ),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .original_text()
            .to_string()
    }

    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .updated_text()
            .to_string()
    }
}

// ── Transform card ────────────────────────────────────────────────────────────

#[pyclass(name = "TransformCard")]
pub struct PyTransform {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PyTransform {
    #[getter]
    fn card_id(&self, py: Python<'_>) -> u32 {
        self.transform_id(py)
    }

    #[setter]
    fn set_card_id(&self, py: Python<'_>, new_id: u32) {
        self.set_transform_id(py, new_id);
    }

    #[getter]
    fn transform_id(&self, py: Python<'_>) -> u32 {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Transform(t) => t.transform_id(),
            _ => unreachable!(),
        }
    }

    #[setter]
    fn set_transform_id(&self, py: Python<'_>, v: u32) {
        let mut m = self.model.borrow_mut(py);
        match &mut m.inner.data_cards[self.index] {
            DataCard::Transform(t) => t.set_transform_id(v),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn starred(&self, py: Python<'_>) -> bool {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Transform(t) => t.starred(),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn coeffs(&self, py: Python<'_>) -> Vec<f64> {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Transform(t) => t.coeffs().to_vec(),
            _ => unreachable!(),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Transform(t) => {
                format!("<TransformCard id={}>", t.transform_id())
            }
            _ => unreachable!(),
        }
    }

    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .original_text()
            .to_string()
    }

    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .updated_text()
            .to_string()
    }
}

// ── Tally card ───────────────────────────────────────────────────────────

#[pyclass(name = "TallyCard")]
pub struct PyTally {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PyTally {
    #[getter]
    fn card_id(&self, py: Python<'_>) -> u32 {
        self.tally_id(py)
    }

    #[setter]
    fn set_card_id(&self, py: Python<'_>, new_id: u32) {
        self.set_tally_id(py, new_id);
    }

    #[getter]
    fn tally_id(&self, py: Python<'_>) -> u32 {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Tally(t) => t.tally_id(),
            _ => unreachable!(),
        }
    }

    #[setter]
    fn set_tally_id(&self, py: Python<'_>, v: u32) {
        let mut m = self.model.borrow_mut(py);
        match &mut m.inner.data_cards[self.index] {
            DataCard::Tally(t) => t.set_tally_id(v),
            _ => unreachable!(),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Tally(t) => format!("<TallyCard id={}>", t.tally_id()),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .original_text()
            .to_string()
    }

    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .updated_text()
            .to_string()
    }
}

// ── Unparsed data card ───────────────────────────────────────────────────────────

#[pyclass(name = "UnparsedCard")]
pub struct PyOther {
    pub model: Py<PyModel>,
    pub index: usize,
}

#[pymethods]
impl PyOther {
    #[getter]
    fn card_id(&self, py: Python<'_>) -> String {
        self.card_name(py)
    }

    #[getter]
    fn card_name(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Other(o) => o.card_name().into(),
            _ => unreachable!(),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        match &m.inner.data_cards[self.index] {
            DataCard::Other(o) => format!("<UnparsedCard {}>", o.card_name()),
            _ => unreachable!(),
        }
    }

    #[getter]
    fn original_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .original_text()
            .to_string()
    }

    #[getter]
    fn updated_text(&self, py: Python<'_>) -> String {
        self.model.borrow(py).inner.data_cards[self.index]
            .updated_text()
            .to_string()
    }
}
