use pyo3::prelude::*;

use crate::py_cell_card::{PyCell, PyCellParameter};
use crate::py_data_cards::{PyMaterial, PyOther, PyTally, PyTransform, PyZaid};
use crate::py_model::PyModel;
use crate::py_surface_card::PySurface;
use crate::py_utils::run;

mod py_cell_card;
mod py_data_cards;
mod py_model;
mod py_surface_card;
mod py_utils;

#[pymodule]
fn _migjorn(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run, m)?)?;
    m.add_class::<PyModel>()?;
    m.add_class::<PyCell>()?;
    m.add_class::<PyCellParameter>()?;
    m.add_class::<PySurface>()?;
    m.add_class::<PyZaid>()?;
    m.add_class::<PyMaterial>()?;
    m.add_class::<PyTransform>()?;
    m.add_class::<PyTally>()?;
    m.add_class::<PyOther>()?;
    Ok(())
}
