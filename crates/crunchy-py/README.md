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
uv sync --extra notebook
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
the usual cause. After changing the Rust code, re-run `uv sync --extra notebook`
(or `maturin develop`) to rebuild.

## Install (development, without uv)

```bash
pip install maturin
cd crates/crunchy-py
maturin develop --release
```

To build a wheel: `maturin build --release` (produces an `abi3` wheel that works
on CPython 3.9+).

## Quick start

```python
import crunchy

deck = crunchy.parse(open("model.mcnp").read())
# or: deck = crunchy.Deck.from_file("model.mcnp")

print(deck)                      # Deck(cells=..., surfaces=..., ...)
print(deck.num_cells, deck.num_surfaces)

# Look up by number (fast; uses an id index).
s = deck.surface(113)
print(s.id, s.kind, s.coeffs)    # 113 'PX' [-10.0]

c = deck.cell(800)
print(c.material, c.density, c.signed_surfaces)

# Diagnostics (empty on a clean parse).
for d in deck.diagnostics:
    print(d.severity, d.message)

# --- Whole-geometry renumbering -------------------------------------------
# Definitions AND every reference are updated consistently.

deck.offset_surfaces(1_000_000)          # fast: shift all surface numbers
deck.renumber_cells(lambda n: n + 500)   # callable: once per distinct cell id
deck.renumber_surfaces({1: 100, 2: 200}) # or an explicit dict

# Lossless re-emission: only edited numbers change; comments/spacing preserved.
deck.save("model_renumbered.mcnp")
text = str(deck)                         # or deck.to_source()
```

## Type checking / IDE support

The package is typed (PEP 561): the compiled extension is the private
`crunchy._crunchy` submodule, wrapped by the `crunchy` package which ships type
stubs (`python/crunchy/__init__.pyi`) and a `py.typed` marker. Pylance, pyright,
and mypy pick these up automatically once the wheel is installed — no more
"unknown import"/"untyped" warnings. When you change the bindings in
`src/lib.rs`, update `python/crunchy/__init__.pyi` to match.

## Notes

- `deck.cells` / `deck.surfaces` / `deck.materials` / `deck.transforms` /
  `deck.data_cards` return full lists. For very large decks prefer the id
  lookups (`deck.cell(id)`, `deck.surface(id)`) and the `num_*` counts.
- Everything is documented inline: `help(crunchy.Deck)`, `help(crunchy.Surface)`.
