# M5 — Editing API & whole-geometry renumbering

## Capability

The CST carries a per-token **override overlay** (`set_token_int` /
`set_token_text`), so any edit rewrites only the affected tokens; every other
byte — comments, spacing, continuations — is re-emitted exactly. Overrides use
`i64` values where possible, so bulk numeric edits allocate nothing per token
(digits are written straight into the output on `to_source`).

`crunchy-core::renumber_surfaces` / `renumber_cells` take a mapping
`old id -> new id` and update **definitions and every reference** consistently:

- surfaces: each surface card id, and every signed surface in cell geometry
  (sense preserved; `+`/white-boundary prefix preserved);
- cells: each cell id, every `#n` complement, and every `LIKE n` base.

References are found with `scan_cell_refs`, an allocation-light token scan that
builds no geometry tree (`GeomExpr` is still available for callers who want the
structure).

## Validation on `resources/untracked/big.mcnp`

Renumber the whole geometry by `+1_000_000` (surfaces and cells), then re-parse
the emitted model:

| Check | Result |
|---|---|
| tokens (before → after) | 91,573,762 → 91,573,762 ✓ |
| cards | 1,018,758 → 1,018,758 ✓ |
| surfaces | 592,588 → 592,588 ✓ |
| surface references checked | 23,668,223 |
| **dangling references** | **0** ✓ |
| min surface id after offset | 1,000,020 (≥ offset ✓) |

Every one of the 23.7M references resolves to a renumbered definition — the edit
is globally consistent and lossless everywhere else.

## Performance

- Renumber (24M coordinated edits): ~9.8 s.
- Emit the 405 MB result: ~7.2 s.

This is the bulk-edit path; ordinary reads (the stated 1M-surfaces-<1s target)
are unaffected and remain sub-second. Progression during M5: 26 s → 17 s after
(a) a fast hasher for the override map (`rustc-hash`), (b) an allocation-light
reference scan replacing per-cell `GeomExpr` construction, and (c) a
coefficient-free surface-id read.

### Remaining optimization (not yet done)

At ~26% of tokens edited, the sparse `HashMap` overlay is doing 24M inserts and
91M lookups on emit. A **sorted `Vec<(token, override)>` merged during emit**
would drop both the hashing and the 91M random lookups (bulk-copy source between
edit points), at the cost of `O(log n)` random access — acceptable, since typed
reads on edited trees are rare. This is the main lever if whole-file renumber
latency needs to come down further.
