# M9 — Renumber generalization (materials, transforms, universes, tallies)

Renumbering now covers every id class, not just cells and surfaces. Each takes a
mapping (dict or callable), like the existing methods.

| Method | Renumbers | Also updates |
|---|---|---|
| `renumber_cells` | cell defs | `#n` complements, `LIKE n`, **cell-tally (F4/F6/F7/F8) bins** |
| `renumber_surfaces` | surface defs | signed surfaces in geometry, **surface-tally (F1/F2) bins** |
| `renumber_materials` | `Mn` defs | every cell's material field, `MTn`/`MXn` cards (void left alone) |
| `renumber_transforms` | `TRn`/`*TRn` defs | each surface's transform field (periodic sign kept) |
| `renumber_universes` | `u=` defs | `fill=` refs, incl. lattice fill arrays; universe 0 kept |
| `renumber_tallies` | `Fn` tally ids | companion cards (`FC`, `FM`, `FS`, `E`, `T`, `C`, `SD`, …) |

```python
model.renumber_materials({6: 106})
model.renumber_transforms(lambda n: n + 100)
model.renumber_universes({5: 500})     # u= and fill= (lattice arrays too)
model.renumber_cells(lambda n: n + 10) # cell defs, #n, LIKE, and F4/F6/.. bins
model.renumber_tallies({4: 24})        # the tally ids themselves
```

## How

All of these follow the existing pass-over-cards + token-override pattern
(`migjorn/src/renumber.rs`), so untouched bytes stay exact and re-emission
is a single linear pass.

- **Mnemonic numbers** (`m6`, `MT6`, `tr3`, `f4`, `e4`, …) are rewritten
  preserving the non-digit prefix and its case, via `rewrite_trailing_number`.
- **Universes** required a real **cell-parameter parser** (`cell_params` in
  `cell.rs`): it splits the parameter tail into `keyword=value…` entries
  (handling `*`-prefixes and `:particle`), from which `u=`/`fill=` values are
  extracted. Lattice fills (`fill= i1:i2 j1:j2 k1:k2  u u u …`) are handled by
  skipping the range triples and renumbering the universe entries after them.
- **Tallies**: a light `parse_tally` reads the `Fn` number and, from its last
  digit, the bin kind (surface 1/2, cell 4/6/7/8, detector 5). The
  cell/surface passes reuse this to renumber the ids inside the bins; sign
  (surface direction) is preserved and 0 is skipped. The hot cell/surface loops
  stay fast because `parse_tally` bails on the card-kind check before allocating
  for the millions of non-data cards.

## Scope notes

- Tally companion coverage is a curated prefix set (the common cards); exotic or
  rarely-used tally cards may not be included.
- `renumber_tallies` changes tally *ids*; the cell/surface ids *inside* bins are
  the job of `renumber_cells`/`renumber_surfaces`.
- As before, don't interleave whole-model renumber with pending structural edits
  on the same cards (structural edits emit from owned nodes, which ignore token
  overrides).
