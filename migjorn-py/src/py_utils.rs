use std::path::PathBuf;

use pyo3::prelude::*;

#[derive(FromPyObject)]
pub enum PathLike {
    #[pyo3(transparent)]
    Path(PathBuf),
    #[pyo3(transparent)]
    String(String),
}

#[pyfunction]
pub fn run(args: Vec<String>) -> i32 {
    migjorn::cli::run(args)
}
