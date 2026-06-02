use clap::Args;
use std::path::PathBuf;

use crate::cli::utils::load_model;

/// Parse a model file and report all unparsed or failed cards
#[derive(Args)]
pub struct CheckArgs {
    /// Path to the MCNP input file
    pub file: PathBuf,
}

pub fn run(args: &CheckArgs) -> i32 {
    match load_model(&args.file) {
        Ok(_) => {
            println!("All cards parsed successfully.");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
