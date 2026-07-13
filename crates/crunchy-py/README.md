# crunchy (Python)

Fast, **lossless** MCNP input parsing with whole-geometry editing, backed by a
Rust core.

## Run the examples with uv (no manual build)

[`uv`](https://docs.astral.sh/uv/) builds the Rust extension for you. From the
repository root:

```bash
# Script tour — one command, builds + runs:
uv run crates/crunchy-py/examples/crunchy_demo.py

# Notebook (interactive) — the [notebook] extra pulls in jupyterlab/ipykernel/nbconvert:
uv run --with "./crates/crunchy-py[notebook]" \
  jupyter lab crates/crunchy-py/examples/crunchy_demo.ipynb

# Notebook (execute headless, refresh outputs in place):
uv run --with "./crates/crunchy-py[notebook]" \
  jupyter nbconvert --to notebook --execute --inplace \
  crates/crunchy-py/examples/crunchy_demo.ipynb
```

The script carries its dependency (the local `crunchy-mcnp` package) as inline
[PEP 723](https://peps.python.org/pep-0723/) metadata, so `uv run` compiles the
extension via maturin into an isolated environment automatically.

## Use in an IDE (VS Code / Jupyter kernel)

To run the notebook against a persistent, selectable interpreter (rather than
uv's ephemeral `uv run` environments), create a `.venv` that contains **both**
`crunchy` and `ipykernel`. From this directory (`crates/crunchy-py`):

```bash
uv sync --no-editable --extra notebook
```

This builds the extension and installs the notebook stack (jupyterlab, ipykernel,
nbconvert) into `crates/crunchy-py/.venv`. Then, in your IDE, select the kernel /
interpreter:

```
crates/crunchy-py/.venv/Scripts/python.exe      # Windows
crates/crunchy-py/.venv/bin/python              # macOS / Linux
```

A kernel without `ipykernel` (e.g. a bare `uv venv`, or `uv sync` **without**
`--extra notebook`) will fail to start for the notebook — that missing package is
the usual cause. After changing the Rust code, re-run
`uv sync --no-editable --extra notebook` (or `maturin develop`) to rebuild.

> **Why `--no-editable`?** An editable install points a `.pth` at the external
> `python/` source dir; Pylance often won't follow that to find the stubs, so you
> get no autocomplete/typing. A non-editable install places `crunchy/__init__.pyi`
> and `py.typed` physically in `site-packages`, which every type checker resolves.
> (Trade-off: after editing the code you must re-sync to pick up changes.)

## Install (development, without uv)

```bash
pip install maturin
cd crates/crunchy-py
maturin develop --release
```

To build a wheel: `maturin build --release` (produces an `abi3` wheel that works
on CPython 3.9+).

To build a Windows wheel on Linux, use `maturin build --release --target x86_64-pc-windows-msvc -m crates/crunchy-py/Cargo.toml -i python3.12`.

## Quick start

```python
import crunchy

model = crunchy.parse(open("model.mcnp").read())
# or: model = crunchy.Model.from_file("model.mcnp")

print(model)                      # Model(cells=..., surfaces=..., ...)
print(model.num_cells, model.num_surfaces)

# Look up by number (fast; uses an id index).
s = model.surface(113)
print(s.id, s.kind, s.coeffs)    # 113 'PX' [-10.0]

c = model.cell(800)
print(c.material, c.density, c.signed_surfaces)

# --- In-place value edits (write straight through, lossless elsewhere) ------
c.material = 124                  # replace the material
c.density  = -7.93               # replace the density

# Assigning a material to a *void* cell just works: it gains a placeholder
# density of 0.0 (never raises, so it's safe in a loop). Set the real density
# next; assigning material 0 makes the cell void again and drops the density.
void = model.cell(2)
void.material = 5                    # 5 0 ...   (placeholder density 0.0)
void.density  = -2.0                # 5 -2.0 ...
void.material = 0                    # 0 ...     (void again, density dropped)

# Diagnostics (empty on a clean parse).
for d in model.diagnostics:
    print(d.severity, d.message)

# --- Whole-geometry renumbering -------------------------------------------
# Definitions AND every reference are updated consistently.

model.offset_surfaces(1_000_000)          # fast: shift all surface numbers
model.renumber_cells(lambda n: n + 500)   # callable: once per distinct cell id
model.renumber_surfaces({1: 100, 2: 200}) # or an explicit dict

# Lossless re-emission: only edited numbers change; comments/spacing preserved.
model.save("model_renumbered.mcnp")
text = str(model)                         # or model.to_source()
```

## Type checking / IDE support

The package is typed (PEP 561): the compiled extension is the private
`crunchy._crunchy` submodule, wrapped by the `crunchy` package which ships type
stubs (`python/crunchy/__init__.pyi`) and a `py.typed` marker. Pylance, pyright,
and mypy pick these up automatically once the wheel is installed — no more
"unknown import"/"untyped" warnings. When you change the bindings in
`src/lib.rs`, update `python/crunchy/__init__.pyi` to match.

## Notes

- `model.cells` / `model.surfaces` / `model.materials` / `model.transforms` /
  `model.data_cards` return full lists. For very large models prefer the id
  lookups (`model.cell(id)`, `model.surface(id)`) and the `num_*` counts.
- Everything is documented inline: `help(crunchy.Model)`, `help(crunchy.Surface)`.
