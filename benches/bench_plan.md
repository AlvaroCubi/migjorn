# Benchmark plan

The rewrite must **prove** it meets [`../docs/04-performance-budget.md`](../docs/04-performance-budget.md).
This file says what to measure, on what input, and how to make it fail CI when a
threshold is missed.

## Inputs

- **Reproducible stand-in**: `python gen_input.py --cells 1000000 > big.mcnp`.
  This targets **card-count parity** with the real model (~1M cards), which is
  the primary driver of structural-edit and per-card-scan cost. Note it is
  ~55 MB (its cards are lighter than the real file's continuation-heavy ones);
  for **byte-size parity** (~380 MB) either raise `--cells` to ~7,000,000 or use
  the real private `big.mcnp`. Report which input a run used.
- **Quick local input**: `--cells 50000` (~3 MB) for fast iteration; thresholds
  scale roughly linearly, so use it for smoke-testing the harness, not for
  sign-off.

## What to measure (one timing per budget row, fresh parse each)

Structural editing mutates state, so **re-parse before every isolated
measurement** — reusing an edited model skews the next timing (this is exactly
how the current-library numbers were taken).

1. `parse` — file bytes → `Model`.
2. `to_source` on the **unedited** model.
3. `to_source` after **K** scattered value edits (K = 1, 100, 10 000) — assert it
   stays ~`unedited` (flat, edit-independent): every card emits by the same
   per-card memcpy whether or not it was edited, so K must not move emit time.
4. `add_cell` single — median of N repeats, each from a fresh parse.
5. `remove_cell` single — same.
6. **Session**: 1000 interleaved add/remove/value edits + reads, total wall time.
   This is the headline workload; it must stay ≤ 1 s.
7. `read after edit`: do one edit, then resolve+read a handle; assert ≤ 1 ms
   (i.e. no reparse hides here).
8. `renumber_cells`, `renumber_surfaces` — pass time **plus** the following
   `to_source`, since emit cost is part of the real operation.
9. `id lookup`: `model.cell(id)` in a tight loop — assert O(1) (flat vs. input
   size).

## Harness shape

- **Rust**: a `--release` example binary (à la the throwaway `bench_pipeline.rs`
  used in the evaluation) for the whole-file timings, printing a table with
  columns `operation | target | measured | pass?`. Exit non-zero on any miss so
  CI breaks. Criterion is fine for the small repeatable ops (single add/remove,
  lookup); it is a poor fit for the multi-second whole-file ops (its sampling
  wants many iterations), so time those manually with `std::time::Instant`.
- **Python**: a thin `pytest -q --benchmark` or a plain script that drives the
  same operations through the PyO3 API, to catch binding-layer overhead (the
  GIL, per-call conversion). The Python session in the `crates/migjorn-py/examples/showcase.py` script
  doubles as an end-to-end smoke test — it should run in interactive time.

## Sign-off criterion

A run on the ~1M-card input prints the full budget table with every row `pass?`
= true. Attach the table to the milestone that claims the performance goal is
met. If a target is intentionally relaxed, change it in `docs/04` and here in the
same commit — never silently.
