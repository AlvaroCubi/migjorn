# 04 — Performance Budget

All numbers are for a **~380 MB, ~1M-card, ~425k-cell** model (`big.mcnp`),
release build, single machine. "Current" is the existing library, measured
directly (see [`02-architecture.md`](02-architecture.md) for how). "Target" is
the contract for the rewrite. These are the thresholds `benches/` enforces.

## The budget

| Operation | Current (measured) | Target | Notes |
|---|---:|---:|---|
| **parse** (file → Model) | 0.96 s | **≤ 1.0 s** (needs parallel per-card build) | includes ~0.9 s per-card copy; parallelize it |
| **emit**, unedited | 0.16 s | **≤ 0.20 s** | concat every card's text (memcpy per card) |
| **emit**, after K edits | 1.7–5.8 s (any edit) | **≤ 0.20 s** | same uniform path; edits don't change emit cost |
| **add_cell** (single) | 2.42 s | **≤ 10 ms** | Vec insert + µs lex; no reparse |
| **remove_cell** (single) | 2.44 s | **≤ 10 ms** | Vec remove; no reparse |
| **1000 structural edits** in a session | ~40 min (extrapolated) | **≤ 1.0 s total** | the headline workload |
| **read after an edit** (handle use) | 1.65 s | **≤ 1 ms** | no materialize/reparse; card list is the state |
| **in-card value edit** (set material, one token) | ~µs (but forces later reparse) | **≤ 5 µs, no deferred reparse** | splice one card's buffer |
| **renumber_cells** (defs + refs + bins) | 0.64 s + 1.71 s emit | **≤ 0.5 s** incl. emit | 427k edits |
| **renumber_surfaces** (24.3M refs) | 2.56 s + 5.78 s emit | **≤ 2.0 s** incl. emit | genuinely 24M token rewrites |
| **id lookup** `model.cell(id)` | O(scan) today | **O(1)** | maintained id→slot index |
| **peak memory** | ~2–3× file | **≤ 3× file** | per-card text + tokens (tokens dominate) |

## Measured after M0/M1 (real `big.mcnp`, 377.7 MB, 1,093,898 cards)

Release build, `cargo test --release --test big_file -- --ignored`. Only the
rows M0/M1 implement are listed; the rest arrive with M2–M4.

| Operation | Target | Measured | Pass |
|---|---:|---:|:--:|
| parse (`Cst`, syntax only) | ≤ 1.0 s | **0.444 s** | ✅ |
| parse + id indices (`Model`) | ≤ 1.0 s | **0.848 s** | ✅ |
| emit, unedited | ≤ 0.20 s | **0.075 s** | ✅ |
| id lookup `model.cell(id)` | O(1) | **27 ns** | ✅ |
| typed read of one cell | ≤ 1 ms | **25 µs** | ✅ |
| round-trip, byte for byte | exact | **exact** | ✅ |

Card census: 425,024 cells / 592,588 surfaces / 75,138 comments / 1,145 data /
1 title / 2 blanks. 62.4M tokens (the tiling scheme in the original design would
have produced 116.6M — see `02-architecture.md`, "Tokens index the card").

Diagnostics on the real model: **3**, all from one genuinely malformed line
(`>11->12 CLOCKWISE WITH RESPECT TO Z`, a stray annotation in the cell block that
is neither a comment nor indented). It is reported, its bytes are preserved, and
the file still round-trips exactly — which is the recoverability contract working
on real input rather than a fixture.

> Note: the private reference model has since been refreshed. The current
> `untracked/big.mcnp` no longer contains the line above (grep confirms it's
> gone) and parses with **zero** diagnostics and 425,023 cells (one fewer than
> the 425,024 above) — same file size to one decimal place, so this was a small
> content fix upstream, not a different file. The card census and diagnostic
> above describe the model as it was at the time of this measurement, not
> today's file; see "Measured on the real `big.mcnp`" further down for current
> numbers.

Two implementation notes that mattered more than expected, both on the
index-building path:

1. Scanning cards **per chunk rather than per card** (one result vector per
   chunk, not per card) cut ~0.45 s of allocation overhead.
2. Looking for `Unknown` tokens by walking every token of every card cost ~0.18 s
   in scattered reads. A `has_unknown` flag set at lex time removed it.

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

The rewrite fixes all three by construction: edits are card-local, every card
owns its exact bytes so emit is always a plain per-card memcpy, and there is
never anything to "materialize."

One cost the rewrite *adds* is at parse: giving every card its own `String` +
`Vec<Token>` copies the file into ~2M small allocations, ~0.9 s single-threaded
(measured). This is the deliberate price of uniform ownership (see
[`02-architecture.md`](02-architecture.md) "Why uniform"). It is the
parallelizable part of parse — split into blocks and build cards on a `rayon`
pool — which is how parse stays within the ≤ 1 s budget. If a profile ever shows
parse latency dominating, the shared-arena/borrow scheme is a localized,
edit-logic-free optimization to add then.

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

The harness ships as `crates/migjorn/examples/bench.rs`:

```
python benches/gen_input.py --cells 1000000 > big.mcnp   # reproducible stand-in
cargo run --release -p migjorn --example bench -- big.mcnp
# or point it at the real private model:
cargo run --release -p migjorn --example bench -- path/to/big.mcnp
```

It discovers real ids from the model, so it runs on any input (the generated
stand-in *or* the real `big.mcnp`), and exits non-zero if any row misses.

## Measured on the real `big.mcnp` (377.7 MB, 425 023 cells, 592 588 surfaces)

Release build, `cargo run --release -p migjorn --example bench -- untracked/big.mcnp`,
96 threads (this machine's full core count). **12 of 13 rows meet target; parse
alone misses.**

| Operation | Target | Measured | Pass |
|---|---:|---:|:--:|
| **parse** | ≤ 1.0 s | **1.04–1.08 s** | ❌ |
| emit, unedited | ≤ 0.20 s | **0.01 s** | ✅ |
| emit, after 1 / 100 / 10 000 edits | ≤ 0.20 s | **0.01 s** (flat) | ✅ |
| add_cell (single) | ≤ 10 ms | **0.60 ms** | ✅ |
| remove_cell (single) | ≤ 10 ms | **0.70 ms** | ✅ |
| 1000-edit session | ≤ 1.0 s | **0.26 s** | ✅ |
| read after edit | ≤ 1 ms | **< 0.001 ms** | ✅ |
| in-card value edit | ≤ 5 µs | **0.05 µs** | ✅ |
| renumber_cells + emit | ≤ 0.5 s | **0.11 s** | ✅ |
| renumber_surfaces + emit | ≤ 2.0 s | **0.70 s** | ✅ |
| id lookup | O(1) | **0.08 µs** | ✅ |

Every structural/edit/emit row that motivated the rewrite passes with wide
margin — the headline is that emit stays flat regardless of edit count, and a
single `add`/`remove` is sub-millisecond where the old engine took ~2.4 s.
`renumber_cells`/`renumber_surfaces` also now pass comfortably (see "Renumber
bottleneck: resolved" below) — the only open row is raw `parse`.

## Renumber bottleneck: resolved

Earlier measurements on this same real model showed `renumber_cells` at 0.61 s
(target ≤ 0.5 s) and `renumber_surfaces` at 4.06 s (target ≤ 2.0 s), with a
write-up here attributing the gap to a sequential, non-`Sync` edit-finding pass
(the renumber map can be a Python callable, which can't be invoked off the GIL
thread) and proposing a parallel-memoized-map fix.

The actual bottleneck turned out to be simpler: `Cst::rewrite_many` built one
heap-allocated `String` per rewritten token — millions of them on a renumber
that touches ~24 M surface references. Switching that replacement text from
`String` to `CompactString` (ids are short enough to stay inline, so building
one never touches the heap) removed the allocation traffic directly, with no
change to the parallelization strategy or renumber semantics. Both rows now
pass with wide margin (0.11 s and 0.70 s above) on the exact continuation-heavy
model that used to miss.

## Known limitation: parse on this machine

Plain `parse` (`Cst` build + typed indexing, no edits) measures **1.04–1.08 s**
against the ≤ 1.0 s target on the real model — a consistent, reproducible ~5–8%
miss across repeated runs of both the ignored `big_file` tests and the
`bench.rs` harness, at the machine's full 96 threads. This is a regression
relative to the 0.56 s measured earlier at 16 threads on different hardware;
more threads did not help, which is consistent with the cost model described
above (§"Why the current numbers are what they are"): parse's cost is ~2M small
per-card allocations, and at high thread counts the global allocator's internal
locking is a plausible ceiling that raw core count can't buy through. This has
not been root-caused yet — a per-thread/arena allocator (e.g. `mimalloc`) is the
first thing to try if it needs closing; until then, treat `parse` as marginal
rather than comfortably in-budget on high-core-count machines.
