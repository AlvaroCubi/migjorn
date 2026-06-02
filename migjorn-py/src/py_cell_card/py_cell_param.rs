use migjorn::{FillData, ParamType};
use pyo3::prelude::*;

use crate::py_model::PyModel;

/// A single cell parameter (IMP:N, VOL, FILL, etc.) within a [`PyCell`].
///
/// Obtained by iterating ``cell.params``. Mutations are reflected immediately
/// in the parent model.
#[pyclass(name = "CellParameter")]
pub struct PyCellParameter {
    pub model: Py<PyModel>,
    pub cell_index: usize,
    pub param_index: usize,
}

#[pymethods]
impl PyCellParameter {
    /// The parameter keyword as it would appear in the input file.
    /// One of ``"IMP:N"``, ``"IMP:P"``, ``"U"``, ``"VOL"``, ``"TMP"``,
    /// ``"LAT"``, ``"FILL"``, ``"*FILL"``.
    #[getter]
    fn param_type(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        match &param.param_type {
            ParamType::ImpN(_) => "IMP:N".to_string(),
            ParamType::ImpP(_) => "IMP:P".to_string(),
            ParamType::U(_) => "U".to_string(),
            ParamType::Vol(_) => "VOL".to_string(),
            ParamType::Tmp(_) => "TMP".to_string(),
            ParamType::Lat(_) => "LAT".to_string(),
            ParamType::Fill(FillData { starred: false, .. }) => "FILL".to_string(),
            ParamType::Fill(FillData { starred: true, .. }) => "*FILL".to_string(),
            ParamType::Other(value) => value.clone(),
        }
    }

    /// The primary scalar value of the parameter.
    /// For ``IMP:N``, ``IMP:P``, ``VOL``, ``TMP`` this is a ``float``.
    /// For ``U``, ``LAT``, ``FILL`` (returns universe ID) this is an ``int``.
    /// For unknown parameters (``Other``), this is the raw parameter string.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyObject {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        match &param.param_type {
            ParamType::ImpN(v) | ParamType::ImpP(v) | ParamType::Vol(v) | ParamType::Tmp(v) => {
                v.into_pyobject(py).unwrap().into_any().unbind()
            }
            ParamType::U(v) | ParamType::Lat(v) => v.into_pyobject(py).unwrap().into_any().unbind(),
            ParamType::Fill(FillData { universe, .. }) => {
                universe.into_pyobject(py).unwrap().into_any().unbind()
            }
            ParamType::Other(value) => value.into_pyobject(py).unwrap().into_any().unbind(),
        }
    }

    /// Set the primary scalar value. The type must match the parameter type.
    #[setter]
    fn set_value(&self, py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut m = self.model.borrow_mut(py);
        let param = &mut m.inner.cells[self.cell_index].params_mut()[self.param_index];
        match &mut param.param_type {
            ParamType::ImpN(val)
            | ParamType::ImpP(val)
            | ParamType::Vol(val)
            | ParamType::Tmp(val) => {
                *val = v.extract::<f64>()?;
            }
            ParamType::U(val) | ParamType::Lat(val) => {
                *val = v.extract::<u32>()?;
            }
            ParamType::Fill(FillData { universe, .. }) => {
                *universe = v.extract::<u32>()?;
            }
            ParamType::Other(value) => {
                *value = v.extract::<String>()?;
            }
        }
        Ok(())
    }

    // --- FILL-specific properties ---

    /// Whether this is a ``*FILL`` (rotation-matrix form). ``None`` for non-FILL params.
    #[getter]
    fn fill_starred(&self, py: Python<'_>) -> Option<bool> {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        if let ParamType::Fill(FillData { starred, .. }) = &param.param_type {
            Some(*starred)
        } else {
            None
        }
    }

    /// The transformation ID inside the ``FILL`` parentheses, if present.
    /// ``None`` for non-FILL params or FILL without a transform.
    #[getter]
    fn fill_transform(&self, py: Python<'_>) -> Option<u32> {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        if let ParamType::Fill(FillData { transform, .. }) = &param.param_type {
            *transform
        } else {
            None
        }
    }

    #[setter]
    fn set_fill_transform(&self, py: Python<'_>, v: Option<u32>) -> PyResult<()> {
        let mut m = self.model.borrow_mut(py);
        let param = &mut m.inner.cells[self.cell_index].params_mut()[self.param_index];
        if let ParamType::Fill(FillData { transform, .. }) = &mut param.param_type {
            *transform = v;
            Ok(())
        } else {
            Err(pyo3::exceptions::PyValueError::new_err(
                "Parameter is not a FILL",
            ))
        }
    }

    /// The coefficient list for ``*FILL`` rotation matrices.
    /// Empty list for non-``*FILL`` params.
    #[getter]
    fn fill_coeffs(&self, py: Python<'_>) -> Vec<f64> {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        if let ParamType::Fill(FillData { coeffs, .. }) = &param.param_type {
            coeffs.as_deref().unwrap_or(&[]).to_vec()
        } else {
            Vec::new()
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let m = self.model.borrow(py);
        let param = &m.inner.cells[self.cell_index].params()[self.param_index];
        match &param.param_type {
            ParamType::ImpN(v) => format!("<CellParameter IMP:N={v}>"),
            ParamType::ImpP(v) => format!("<CellParameter IMP:P={v}>"),
            ParamType::U(v) => format!("<CellParameter U={v}>"),
            ParamType::Vol(v) => format!("<CellParameter VOL={v}>"),
            ParamType::Tmp(v) => format!("<CellParameter TMP={v}>"),
            ParamType::Lat(v) => format!("<CellParameter LAT={v}>"),
            ParamType::Fill(FillData {
                starred,
                universe,
                transform,
                ..
            }) => {
                let kw = if *starred { "*FILL" } else { "FILL" };
                match transform {
                    Some(t) => format!("<CellParameter {kw}={universe} ({t})>"),
                    None => format!("<CellParameter {kw}={universe}>"),
                }
            }
            ParamType::Other(value) => format!("<CellParameter {value}>"),
        }
    }
}
