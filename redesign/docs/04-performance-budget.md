# 04 — Performance Budget

All numbers are for a **~380 MB, ~1M-card, ~425k-cell** model (`big.mcnp`),
release build, single machine. "Current" is the existing library, measured
directly (see [`02-architecture.md`](02-architecture.md) for how). "Target" is
the contract for the rewrite. These are the thresholds `benches/` enforces.

## The budget

| Operation | Current (measured) | Target | Notes |
|---|---:|---:|---|
| **parse** (file → Model) | 0.96 s | **≤ 1.0 s** (stretch ≤ 0.5 s parallel) | the only unavoidable O(file) step |
| **emit**, unedited | 0.16 s | **≤ 0.20 s** | memcpy of the snapshot |
| **emit**, after K edits | 1.7–5.8 s (any edit) | **≤ 0.20 s + O(K)** | untouched cards stay memcpy |
| **add_cell** (single) | 2.42 s | **≤ 10 ms** | Vec insert + µs lex; no reparse |
| **remove_cell** (single) | 2.44 s | **≤ 10 ms** | Vec remove; no reparse |
| **1000 structural edits** in a session | ~40 min (extrapolated) | **≤ 1.0 s total** | the headline workload |
| **read after an edit** (`view()`/handle use) | 1.65 s | **≤ 1 ms** | no materialize/reparse |
| **in-card value edit** (set material, one token) | ~µs (but forces later reparse) | **≤ 5 µs, no deferred reparse** | local override |
| **renumber_cells** (defs + refs + bins) | 0.64 s + 1.71 s emit | **≤ 0.5 s** incl. emit | 427k edits |
| **renumber_surfaces** (24.3M refs) | 2.56 s + 5.78 s emit | **≤ 2.0 s** incl. emit | genuinely 24M token rewrites |
| **id lookup** `model.cell(id)` | O(scan) today | **O(1)** | maintained id→slot index |
| **peak memory** | ~2–3× file | **≤ 3× file** | snapshot + arena + card list |

The single non-negotiable: **no editing operation may re-lex or re-parse the
whole file.** If any add/remove/edit/renumber path calls the lexer on more than
the changed card(s), the design has regressed to the current one.

## Why the current numbers are what they are (so we don't repeat them)

1. **Structural edits reparse the file.** `add_cell`/`remove_cell` do
   `to_source()` (emit whole file) then `parse()` (re-lex whole file). ~2.4 s,
   and it does not amortize — 10 edits = 24 s. Root cause: a single flat token
   arena spanning the file, so a card boundary change shifts all token indices.
2. **A read after a splice reparses the file.** Splices are emit-only, so the
   next raw-token read calls `materialize()` = reparse. ~1.65 s.
3. **Any edit disables the whole-file emit fast path.** `to_source()` memcpy's
   only when overrides/splices are globally empty; otherwise it walks ~100M
   tokens doing 2–3 hash lookups each. This is why even a modest renumber emits
   in 1.7 s+.

The rewrite fixes all three by construction: edits are card-local, so untouched
cards stay pristine and their emit stays a memcpy, and there is never anything to
"materialize."

## How to measure (the harness the rewrite must ship)

`benches/` must contain a benchmark that:
1. Loads a large input — either the untracked `big.mcnp`, or one produced by
   `benches/gen_input.py --cells 1000000 > big.mcnp` (so the bench is
   reproducible without the private file).
2. Times each row of the budget table in isolation, from a **fresh parse** per
   measurement (an edit mutates state, so reuse would skew the next timing).
3. Asserts each measurement against its threshold and **fails CI if exceeded**,
   with the actual number printed. Treat a regression as a build break.

Suggested tooling: a criterion bench for the small/repeatable operations, plus a
plain `--release` example binary (like the throwaway `bench_pipeline.rs` used
during this evaluation) for the whole-file timings that are too slow/allocation-
heavy for criterion's sampling. Print a table; compare to this doc.

## Reporting

A perf run should print a table shaped like the budget above with
`current_target / measured / pass?` columns, so a reviewer can see at a glance
whether the rewrite meets its contract. Keep this doc's table and the bench in
sync — if a target changes, change both in the same commit.
