# M8 — Programmatic construction (add/remove cards)

## Capability

Whole cards can now be created and removed. New cards are given as MCNP text;
migjorn validates, appends them to the right block, and returns a **live handle**
that behaves like any other (readable and editable). Removal deletes a card by
number, preserving the surrounding structure.

```python
m.add_surface("3 SO 30")
m.add_material("m7 1001 2 8016 1")
c = m.add_cell("10 7 -1.0 -3 2")   # live handle
c.add_surface(-1)                  # edit the freshly-added cell

m.remove_surface(3)                # -> bool
problems = m.validate()            # ['cell 10 references missing surface 3', ...]
```

- `add_cell(text) -> Cell`, `add_surface(text) -> Surface`,
  `add_material(text) -> Material` — each appends to the end of its block and
  raises `ValueError` if the text is not exactly one well-formed card of that
  kind.
- `remove_cell(id) -> bool`, `remove_surface(id) -> bool` — remove the card's
  physical line(s); blank-line block delimiters and neighbouring comments stay
  intact. Returns `False` if no such card exists.
- `validate() -> list[str]` — referential-integrity check (dangling
  surface/cell/material references).

## How it works: splice + reparse + slot remap

Rather than splicing tokens into the flat CST (which would shift every later
token index), a structural card edit works at the source level:

1. Emit the current model (`to_source`, applying all pending value/geometry
   edits), then splice the new card's text into the target block — or cut a
   removed card's lines — using token byte spans to place it precisely (after the
   last card of the block, before the block's blank delimiter).
2. **Reparse** the spliced source into a fresh tree.
3. **Remap slots**: carried-over cards keep their stable slot; an inserted card
   gets a fresh one (from a monotonic counter that survives reparses). So handles
   held across a structural edit still resolve.

Because the reparse "flattens" any pending value/geometry edits into real tokens,
an added card is immediately a first-class card — indexed by number
(`model.cell(id)`), addressable by slot, and editable via the Stage-B geometry
API. New cards' own formatting is whatever the emitter/text produced; every
untouched card stays byte-for-byte.

## Cost & limits

- A structural card edit is `O(model size)` (emit + reparse), unlike the
  in-place value/geometry edits. Fine for construction; don't call it in a tight
  loop over a very large model if you can batch.
- `add_*` appends to a block that already has at least one card of that kind
  (`EditError::NoBlock` otherwise).
- As in M7, don't interleave whole-model `renumber_*` with pending structural
  edits on the same cards without re-reading.
