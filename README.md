# Migjorn

Migjorn is a toolkit for reading, validating, inspecting, and transforming MCNP input models, with both a native CLI and Python bindings. The project is designed for workflows where performance, reproducibility, and programmatic access all matter: fast parsing in Rust, command-line automation for batch processing, and a Python API for analysis notebooks and scripts.

## What Migjorn Provides

Migjorn focuses on the parts of MCNP model handling that are commonly needed in model maintenance and preprocessing pipelines:

- Parse MCNP input files into structured in-memory objects.
- Very fast multicore parsing --> The 6.5 million lines E-Lite model parsed in 900 ms.
- Inspect model-level statistics and card collections.
- Run validation checks to identify consistency issues.
- Renumber model identifiers safely and write updated models.
- Access the same core capabilities from Python.

## Installation

### Rust CLI (crates.io)

```bash
cargo install migjorn
```

### Python package (PyPI)

```bash
pip install migjorn
```

After installation, both `import migjorn` and the `migjorn` command are available.


## Python API Quick Start

### Loading and inspecting a model

```python
from migjorn import Model

model = Model.from_file("input.i")
print(model.title)
print(len(model.cells), len(model.surfaces), len(model.materials))
```

### Renumbering from Python

```python
model.renumber_cells(100, range=(1, 999))
model.renumber_surfaces(-50)
model.write_to_file("renumbered.i")
```

### Running validation checks

```python
try:
    model.validation_checks()
    print("Model is valid")
except ValueError as e:
    print("Validation report:")
    print(e)
```

## CLI Usage

General help:

```bash
migjorn --help
```

### 1) info

Purpose: parse an MCNP file and print model statistics.

```bash
migjorn info INPUT_FILE
```

Output includes counts of cells, surfaces, universes, and materials, followed by a validation summary.

### 2) parsing-check

Purpose: verify that the file parses cleanly.

```bash
migjorn parsing-check INPUT_FILE
```

Prints `All cards parsed successfully.` on success, or an error to stderr with a non-zero exit code on failure.

### 3) renumber

Purpose: renumber one or more ID domains and write a new output model.

```bash
migjorn renumber INPUT_FILE OUTPUT_FILE [OPTIONS]
```

Main options:

- `--cells OFFSET` with optional `--cell-range FROM TO`
- `--surfaces OFFSET` with optional `--surface-range FROM TO`
- `--materials OFFSET` with optional `--material-range FROM TO`
- `--transformations OFFSET` with optional `--transformation-range FROM TO`
- `--universes OFFSET` with optional `--universe-range FROM TO`

Notes:

- Offsets can be negative.
- Range bounds are inclusive.
- If no range is provided for a selected domain, the offset applies to all IDs in that domain.

Example:

```bash
migjorn renumber input.i output.i --cells 1000 --cell-range 1 999 --surfaces -200
```

## Repository Layout

The repository is a Rust workspace with two crates and a Python package:

- `migjorn`: parser, data model, validation, renumbering logic, and CLI — all in one crate.
- `migjorn-py`: PyO3 extension crate exposing Rust functionality to Python.
- `python/migjorn`: Python package and type stubs, including the CLI entry point.

Additional folders:

- `resources`: sample and stress-test MCNP inputs.
- `python/tests` and `migjorn/tests`: automated test suites.

## Architecture Overview

Migjorn uses a two-layer design:

1. **Core + CLI layer (`migjorn` crate)**
   - Defines card and model types: `Model`, `CellCard`, `SurfaceCard`, `MaterialCard`, `TransformCard`, `TallyCard`, and `UnparsedCard`.
   - Handles parsing, error reporting, validation checks, and serialization back to MCNP-like text.
   - Provides renumbering operations for cells, surfaces, materials, transformations, and universes.
   - Exposes CLI commands (`info`, `parsing-check`, `renumber`) via `migjorn::cli`.

2. **Python binding layer (`migjorn-py` + `python/migjorn`)**
   - Exposes Rust classes and functions through a Python-friendly API.
   - Ships typing information via `.pyi` files.
   - Provides a `python -m migjorn` and console-script entry point that delegates to the Rust CLI logic.

## Building Locally

### Rust CLI

```bash
cargo build --release
```

The executable is at `target/release/migjorn`.

### Python package (development install)

This project uses [maturin](https://www.maturin.rs/) and PyO3 (ABI3, Python 3.8+ compatibility target).

```bash
pip install maturin
maturin develop
```

Wheel build:

```bash
maturin build --release
```

## Error Handling and Exit Codes

CLI behavior is script-friendly:

- `0` on success.
- `1` for runtime failures (file I/O, parse errors).
- `2` for command-line usage errors.

Python API raises exceptions (`IOError`, `ValueError`, etc.) instead of exit codes.

## Testing

```bash
cargo test --workspace
pytest
```

## Typical Workflow

1. Run `parsing-check` to fail fast on malformed files.
2. Use `info` to get quick structural stats and a validation summary.
3. Apply `renumber` operations to avoid ID collisions across merged models.
4. Re-run `info` or validation checks on the output.
5. Use the Python API for custom transformations and reporting when needed.

## Scope and Current Status

Migjorn already provides a strong base for parsing, inspection, validation, and ID-renumbering tasks. Some MCNP data-card families are intentionally represented as `UnparsedCard` when no dedicated parser is implemented yet. This is a deliberate compatibility choice: unknown cards round-trip through the model without being lost.

As parser coverage expands, those cards can be upgraded to typed representations without changing the overall architecture.

