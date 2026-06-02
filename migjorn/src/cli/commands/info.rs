use clap::Args;
use std::path::PathBuf;

use crate::cli::utils::load_model;

/// Parse a model file and print statistics
#[derive(Args)]
pub struct InfoArgs {
    /// Path to the MCNP input file
    pub file: PathBuf,
}

pub fn run(args: &InfoArgs) -> i32 {
    let model = match load_model(&args.file) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    let cells = model.cells.len();
    let surfaces = model.surfaces.len();
    let universes = model.universe_ids().len();
    let materials = model.material_ids().len();

    println!("Cells:             {cells}");
    println!("Surfaces:          {surfaces}");
    println!("Universes:         {universes}");
    println!("Materials:         {materials}");

    match model.validation_checks() {
        Ok(()) => println!("The model passed the validation checks!"),
        Err(report) => println!("{report}"),
    }

    0
}
