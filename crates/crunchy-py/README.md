# crunchy (Python)

Fast, **lossless** MCNP input parsing with whole-geometry editing, backed by a
Rust core.

## Install (development)

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

## Notes

- `deck.cells` / `deck.surfaces` / `deck.materials` / `deck.transforms` /
  `deck.data_cards` return full lists. For very large decks prefer the id
  lookups (`deck.cell(id)`, `deck.surface(id)`) and the `num_*` counts.
- Everything is documented inline: `help(crunchy.Deck)`, `help(crunchy.Surface)`.
