# M7 — Structural geometry editing

## Capability

A cell's geometry can now be **restructured** — surfaces and `#n` complements
added or removed — not just its numeric values. The edited card is re-emitted
from a typed node; every other card stays byte-for-byte.

```python
c = model.cell(1)
c.add_surface(-4)      # intersect the region with surface 4, negative sense
c.add_complement(2)    # AND in  #2
c.remove_surface(4)    # drop every reference to surface 4 (either sense)
c.remove_complement(2) # drop the #2 complement
```

`remove_surface` / `remove_complement` return whether anything was removed and
raise `ValueError` if the edit would leave the cell with no geometry.

## How it works: the hybrid card overlay

The lossless engine still can't insert or delete tokens, so structural edits use
a second overlay at **card granularity**:

- On first structural edit a cell is **promoted** to an owned, typed node
  (`OwnedCell` in `crunchy-core`): `{ id, material, density, geometry, params_text }`.
  `geometry` is the editable `GeomExpr` boolean tree; `params_text` is the
  verbatim tail (`imp`, `u`, `fill`, ...) captured from the source so editing the
  geometry never drops it.
- Editing mutates the `GeomExpr` tree and re-emits the card body via the
  **card emitter** (`crunchy-core::emit`), which honours MCNP precedence
  (`#` tightest, juxtaposition intersection, `:` union) and parenthesises where
  needed.
- The emitted text is stored in the CST as a `slot -> text` **card replacement**
  (`GreenTree::replace_card_content`). On `to_source`, only that card's
  meaningful token span is substituted; leading indentation and trailing trivia
  — newlines, the block's blank delimiter, inline `$` comments, absorbed comment
  lines — are preserved. So a restructured card loses only its own interior
  formatting; the rest of the model is byte-for-byte.

Reads go through `Model::read_cell`, which prefers the owned node when present,
so an edit is immediately visible via any handle (`cell.signed_surfaces`,
`cell.cell_refs`, `cell.text`).

## Value edits stay consistent

`cell.material = …` / `cell.density = …` route through the owned node once a cell
has been promoted, so mixing value and structural edits on the same cell stays
coherent. On an un-promoted cell they still take the fast in-place token path.

## Known limits (future milestones)

- **Whole-model renumber vs. a structurally-edited cell**: renumbering rewrites
  *tokens*; a promoted cell is emitted from its owned node and ignores those
  token overrides, so don't mix `renumber_*` with structural edits on the same
  cell yet.
- Only cell geometry is restructurable so far. Adding/removing whole cards
  (construction) and editable surfaces are the next milestone (the emitter and
  the card overlay are the reusable pieces they build on).
