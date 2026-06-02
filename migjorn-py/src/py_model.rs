use pyo3::prelude::*;
use pyo3::types::PyDict;

use std::path::PathBuf;

use migjorn::{DataCard, Model};

use crate::py_cell_card::PyCell;
use crate::py_data_cards::{PyMaterial, PyOther, PyTally, PyTransform};
use crate::py_surface_card::PySurface;
use crate::py_utils::PathLike;

#[derive(Clone)]
#[pyclass(name = "Model")]
pub struct PyModel {
    pub inner: Model,
}

#[pymethods]
impl PyModel {
    #[new]
    fn new(text: String) -> PyResult<Self> {
        Self::from_text(text)
    }

    #[staticmethod]
    pub fn from_file(path: PathLike) -> PyResult<Self> {
        let path = match path {
            PathLike::Path(p) => p,
            PathLike::String(s) => PathBuf::from(s),
        };
        let inner = Model::from_file(&path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        Ok(PyModel { inner })
    }

    #[staticmethod]
    pub fn from_text(text: String) -> PyResult<Self> {
        let path = PathBuf::from("<string>");
        let inner = Model::from_text(&path, &text)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(PyModel { inner })
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyDict>) -> Self {
        self.clone()
    }

    pub fn write_to_file(&self, path: PathLike) -> PyResult<()> {
        let path = match path {
            PathLike::Path(p) => p,
            PathLike::String(s) => PathBuf::from(s),
        };
        self.inner
            .write_to_file(&path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "<Model cells={} surfaces={} data_cards={}>",
            self.inner.cells.len(),
            self.inner.surfaces.len(),
            self.inner.data_cards.len(),
        )
    }

    #[getter]
    pub fn title(&self) -> String {
        self.inner.title.as_str().to_string()
    }

    #[getter]
    pub fn cells(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PyCell>> {
        let count = slf.borrow().inner.cells.len();
        let model_py = slf.clone().unbind();
        (0..count)
            .map(|index| {
                Py::new(
                    py,
                    PyCell {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    pub fn surfaces(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PySurface>> {
        let count = slf.borrow().inner.surfaces.len();
        let model_py = slf.clone().unbind();
        (0..count)
            .map(|index| {
                Py::new(
                    py,
                    PySurface {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    pub fn materials(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PyMaterial>> {
        let model_py = slf.clone().unbind();
        Self::data_card_indices(slf, |c| matches!(c, DataCard::Material(_)))
            .into_iter()
            .map(|index| {
                Py::new(
                    py,
                    PyMaterial {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    pub fn transforms(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PyTransform>> {
        let model_py = slf.clone().unbind();
        Self::data_card_indices(slf, |c| matches!(c, DataCard::Transform(_)))
            .into_iter()
            .map(|index| {
                Py::new(
                    py,
                    PyTransform {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    pub fn tallies(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PyTally>> {
        let model_py = slf.clone().unbind();
        Self::data_card_indices(slf, |c| matches!(c, DataCard::Tally(_)))
            .into_iter()
            .map(|index| {
                Py::new(
                    py,
                    PyTally {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[getter]
    pub fn unparsed_data_cards(slf: &Bound<'_, Self>, py: Python<'_>) -> Vec<Py<PyOther>> {
        let model_py = slf.clone().unbind();
        Self::data_card_indices(slf, |c| matches!(c, DataCard::Other(_)))
            .into_iter()
            .map(|index| {
                Py::new(
                    py,
                    PyOther {
                        model: model_py.clone_ref(py),
                        index,
                    },
                )
                .unwrap()
            })
            .collect()
    }

    #[pyo3(signature = (offset, range=None))]
    fn renumber_cells(&mut self, offset: i32, range: Option<(u32, u32)>) {
        let r = match range {
            Some((start, stop)) => start..=stop,
            None => u32::MIN..=u32::MAX,
        };
        self.inner.renumber_cells(r, offset);
    }

    #[pyo3(signature = (offset, range=None))]
    fn renumber_surfaces(&mut self, offset: i32, range: Option<(u32, u32)>) {
        let r = match range {
            Some((start, stop)) => start..=stop,
            None => u32::MIN..=u32::MAX,
        };
        self.inner.renumber_surfaces(r, offset);
    }

    #[pyo3(signature = (offset, range=None))]
    fn renumber_materials(&mut self, offset: i32, range: Option<(u32, u32)>) {
        let r = match range {
            Some((start, stop)) => start..=stop,
            None => u32::MIN..=u32::MAX,
        };
        self.inner.renumber_materials(r, offset);
    }

    #[pyo3(signature = (offset, range=None))]
    fn renumber_transformations(&mut self, offset: i32, range: Option<(u32, u32)>) {
        let r = match range {
            Some((start, stop)) => start..=stop,
            None => u32::MIN..=u32::MAX,
        };
        self.inner.renumber_transformations(r, offset);
    }

    #[pyo3(signature = (offset, range=None))]
    fn renumber_universes(&mut self, offset: i32, range: Option<(u32, u32)>) {
        let r = match range {
            Some((start, stop)) => start..=stop,
            None => u32::MIN..=u32::MAX,
        };
        self.inner.renumber_universes(r, offset);
    }

    fn validation_checks(&self) -> PyResult<()> {
        self.inner
            .validation_checks()
            .map_err(|report| pyo3::exceptions::PyValueError::new_err(report.to_string()))
    }
}

impl PyModel {
    fn data_card_indices(
        slf: &Bound<'_, Self>,
        predicate: impl Fn(&DataCard) -> bool,
    ) -> Vec<usize> {
        slf.borrow()
            .inner
            .data_cards
            .iter()
            .enumerate()
            .filter_map(|(i, c)| predicate(c).then_some(i))
            .collect()
    }
}
