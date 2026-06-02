use clap::Args;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::utils::load_model;

/// Renumber cells, surfaces, materials, transformations, and universes in a model
#[derive(Args)]
#[command(allow_negative_numbers = true)]
pub struct RenumberArgs {
    /// Path to the MCNP input file
    pub file: PathBuf,
    /// Path to write the output file
    pub output: PathBuf,
    /// Offset to apply to cell IDs (all cells unless --cell-range is given)
    #[arg(short, long)]
    pub cells: Option<i32>,
    /// Restrict cell renumbering to IDs in this inclusive range [FROM TO]
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], requires = "cells", value_parser = parse_u32)]
    pub cell_range: Option<Vec<u32>>,
    /// Offset to apply to surface IDs (all surfaces unless --surface-range is given)
    #[arg(short, long)]
    pub surfaces: Option<i32>,
    /// Restrict surface renumbering to IDs in this inclusive range [FROM TO]
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], requires = "surfaces", value_parser = parse_u32)]
    pub surface_range: Option<Vec<u32>>,
    /// Offset to apply to material IDs (all materials unless --material-range is given)
    #[arg(short, long)]
    pub materials: Option<i32>,
    /// Restrict material renumbering to IDs in this inclusive range [FROM TO]
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], requires = "materials", value_parser = parse_u32)]
    pub material_range: Option<Vec<u32>>,
    /// Offset to apply to transformation IDs (all transformations unless --transformation-range is given)
    #[arg(short, long)]
    pub transformations: Option<i32>,
    /// Restrict transformation renumbering to IDs in this inclusive range [FROM TO]
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], requires = "transformations", value_parser = parse_u32)]
    pub transformation_range: Option<Vec<u32>>,
    /// Offset to apply to universe IDs (all universes unless --universe-range is given)
    #[arg(short, long)]
    pub universes: Option<i32>,
    /// Restrict universe renumbering to IDs in this inclusive range [FROM TO]
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], requires = "universes", value_parser = parse_u32)]
    pub universe_range: Option<Vec<u32>>,
}

pub fn run(args: &RenumberArgs) -> i32 {
    let mut model = match load_model(&args.file) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    if let Some(offset) = args.cells {
        let timer = Instant::now();
        model.renumber_cells(range_or_all(args.cell_range.clone()), offset);
        println!("Renumbered cells in {:?}", timer.elapsed());
    }

    if let Some(offset) = args.surfaces {
        let timer = Instant::now();
        model.renumber_surfaces(range_or_all(args.surface_range.clone()), offset);
        println!("Renumbered surfaces in {:?}", timer.elapsed());
    }

    if let Some(offset) = args.materials {
        let timer = Instant::now();
        model.renumber_materials(range_or_all(args.material_range.clone()), offset);
        println!("Renumbered materials in {:?}", timer.elapsed());
    }

    if let Some(offset) = args.transformations {
        let timer = Instant::now();
        model.renumber_transformations(range_or_all(args.transformation_range.clone()), offset);
        println!("Renumbered transformations in {:?}", timer.elapsed());
    }

    if let Some(offset) = args.universes {
        let timer = Instant::now();
        model.renumber_universes(range_or_all(args.universe_range.clone()), offset);
        println!("Renumbered universes in {:?}", timer.elapsed());
    }

    let timer = Instant::now();
    match model.write_to_file(&args.output) {
        Ok(()) => println!("Wrote {} in {:?}", args.output.display(), timer.elapsed()),
        Err(e) => {
            eprintln!("Error writing {}: {e}", args.output.display());
            return 1;
        }
    }

    0
}

fn parse_u32(s: &str) -> Result<u32, String> {
    let normalized = s.replace('_', "");
    if let Ok(v) = normalized.parse::<u32>() {
        return Ok(v);
    }
    match normalized.parse::<f64>() {
        Ok(f) if f >= 0.0 && f <= u32::MAX as f64 && f.fract() == 0.0 => Ok(f as u32),
        Ok(f) => Err(format!(
            "'{s}' is out of range or not a whole number (got {f})"
        )),
        Err(_) => Err(format!("'{s}' is not a valid integer")),
    }
}

fn range_or_all(range: Option<Vec<u32>>) -> RangeInclusive<u32> {
    match range {
        Some(r) => r[0]..=r[1],
        None => 1..=u32::MAX,
    }
}
