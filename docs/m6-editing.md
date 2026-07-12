# M6 — Per-card exploration & value editing

## Capability

Typed Python objects are now **live handles** onto their model, not detached
snapshots. Each `Cell`/`Surface`/`Material`/`Transform` holds a reference to its
`Model` plus a stable **card slot**; getters read the current card on demand, and
setters write straight through the lossless override overlay.

```python
import crunchy
model = crunchy.parse(text)

for cell in model.cells:
    if "$ vacuum vessel" in cell.text:   # raw card source, incl. inline $ comments
        cell.material = 124              # in-place value edits, visible immediately
        cell.density  = 7.93

print(model.to_source())                 # edited tokens changed; everything else byte-identical
```

New surface area:

- **`cell.text` / `surface.text` / `material.text` / `transform.text`** — the
  card's exact source span (inline `$` comments and `&`/indent continuations
  included), reflecting any edits. Backed by `GreenTree::card_source`.
- **`cell.material = …` / `cell.density = …`** — value edits that replace the
  existing token. Backed by `core::Model::set_cell_material` /
  `set_cell_density`.
- **`num_materials` / `num_transforms`** counts, alongside the existing
  `num_cells` / `num_surfaces`.

## The replace-only boundary

The override overlay can only *replace* an existing token, never insert or delete
one. So value edits succeed only when the target token already exists; edits that
would change a card's token layout raise `ValueError` (Rust `EditError`):

| Attempted edit | Result |
|---|---|
| `cell.material = N` (real → real) | ✓ replaces the material token |
| `cell.density = X` (non-void cell) | ✓ replaces the density token |
| `cell.density = X` on a void cell | ✗ `NoDensityField` (would add a token) |
| `cell.material = 0` on a real cell | ✗ `VoidnessChange` (would remove density) |
| `cell.material = N` on a void cell | ✗ `VoidnessChange` (would add density) |
| material/density on a `LIKE n BUT` cell | ✗ `NoMaterialField` |

Structural edits (adding/removing surfaces, params, or whole cards) are the next
milestone; they re-emit a card from a typed node rather than patching tokens.

## Stable card slots

`GreenTree` now assigns every card a monotonic **slot** (`card_slot(pos)`,
`card_by_slot(slot)`), never reused for the life of the tree. Python handles key
on the slot, so once structural insert/delete lands (M7+), inserting or deleting
cards will not invalidate handles you already hold. Today (value edits only) slots
equal positions; the reverse map is rebuilt whenever the card list changes.

## Notes

- Getters re-parse their (tiny) card per access, trading a little work for
  liveness: an edit through one handle is visible through any other handle or a
  fresh `model.cell(id)` lookup.
- Re-emission stays byte-for-byte for every untouched token, so a value edit
  changes only the numbers you set.
