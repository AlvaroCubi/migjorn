# 05 — Parallelism overhead on multi-model workloads

`04-performance-budget.md` benchmarks `migjorn` in isolation, one operation on
one big file at a time. This doc covers a different shape of workload: a
caller that drives `migjorn` over **many models in one process** — this is
exactly what `gitronics` (`build`) does: parse ~170 filler files, clear their
data blocks, merge them into one envelope structure, validate, re-parse,
write. Profiling that real workload (`big_example`, 172 unique fillers, a
~10M-line composed model, 96-core machine) surfaced a problem `04` had already
flagged but not root-caused: **more threads don't help, and past a point they
actively hurt.**

## The finding: real time is flat, sys time is not

Same binary, same input, only `RAYON_NUM_THREADS` varied:

| Threads | real | user | sys |
|---:|---:|---:|---:|
| 1 | 14.4 s | 10.4 s | 3.9 s |
| 4 | 11.3 s | 10.8 s | 5.9 s |
| 8 | 11.1 s | 11.0 s | 10.5 s |
| 16 | 12.2 s | 11.5 s | 15.6 s |
| 32 | 10.7 s | 13.1 s | 25.8 s |
| 64 | 11.4 s | 19.6 s | 64.0 s |
| 96 (default = num_cpus) | 12.3 s | 31.0 s | 97.0 s |

Wall-clock time is within noise across the whole range (10.7–14.4 s). `sys`
time — kernel time, not compute — grows monotonically and dramatically with
thread count, reaching **97 s at the default pool size**, ~8× the wall-clock
time of the run it belongs to. This machine's default rayon pool (one thread
per logical CPU, 96) is pure waste for this workload: it buys zero real-time
benefit and burns almost 10 CPU-minutes of scheduler/futex overhead per build.

This is the same symptom `04`'s "Known limitation: parse on this machine"
section describes for the single-big-file `bench.rs` case ("more threads did
not help... at high thread counts the global allocator's internal locking is
a plausible ceiling"), but the evidence here points more specifically at
**parallel-dispatch overhead**, not (only) allocator locking: `sys` time
scales smoothly with thread count even though the per-thread workload doesn't
shrink, which is the signature of threads being woken and parked
(`futex`) far more often than they do useful work, not of a shared lock
under contention. `mimalloc`/`jemalloc` (04's suggested next step) is still
worth trying, but it will not fix the mechanism identified below.

## Root cause: nested parallelism at the wrong granularity

Stage breakdown of the same build (default 96 threads):

| Stage | Wall time | Parallel today? |
|---|---:|:--:|
| `validate()` on the assembled model | 3.68 s | **No** |
| `merge()` | 1.86 s | No |
| 172× `clear_data_cards()` on fillers | 1.46 s | No (`.iter()`) |
| `load_fillers` parse, 172 files | 0.77 s | Yes (outer, gitronics) |
| final assembled-model re-parse | 0.32 s | Yes (`build_indices`) |
| write output | 0.42 s | No |

`gitronics::load_fillers` parallelizes **across files** (one rayon task per
filler, ~170-way). Inside each of those tasks, `Model::parse` →
`build_indices` **also** parallelizes, via `par_chunks` sized off
`rayon::current_num_threads() * 4` — i.e. up to ~384-way on this machine. Both
dispatch onto the same global pool, so a single `load_fillers` call produces
on the order of 170 outer tasks × up to 384 inner sub-tasks each, all
contending for 96 OS threads to do work that, per file, is a few hundred
KB–MB of text — too small a grain to need that much fan-out. The 172
sequential `clear_data_cards()` calls make it worse: each one re-runs
`Model::parse` (full re-lex + a fresh `build_indices` fan-out) from scratch,
so the same nested-dispatch cost is paid a second time per filler, serially.

This is invisible on `04`'s single-big-file benchmark because there's only
one `build_indices` call in flight at a time. It only shows up when a caller
composes many models in one process — which is `migjorn`'s actual primary
consumer (`gitronics`), not an edge case.

## Required improvements

Ranked by expected impact on the profiled workload. **Status: 1-5 have since
shipped**, except the "better" half of (1) (a bounded/caller-supplied pool);
see the note after each item.

1. **Stop sizing internal parallelism off raw `num_cpus`.** `build_indices`'
   chunk count (`current_num_threads() * 4`) assumes it owns the whole pool;
   it doesn't when called from inside a caller's own `par_iter`. At minimum,
   document that a caller doing file-level parallelism (like `gitronics`)
   should cap the global pool (`RAYON_NUM_THREADS` or
   `ThreadPoolBuilder::num_threads(N).build_global()`) rather than accept the
   default — this alone drops the 97 s of `sys` time to ~11–26 s (8–32
   threads) with no real-time cost. Better: give `migjorn` a way to run its
   internal parallel steps against a bounded/caller-supplied pool instead of
   always reaching for the global one, so a multi-model caller doesn't have
   to reason about this at all.

   **Documented** (see `crates/migjorn/src/lib.rs`'s crate-level doc comment,
   which tells a file-level-parallel caller to cap the global pool). The
   "better" bounded/caller-supplied-pool API is still not built — the
   documented workaround is the only fix so far.
2. **Threshold-gate `build_indices`' inner parallelism.** Below some card
   count (needs a measurement to pick, but the 172 fillers here — a few
   hundred to tens of thousands of cards each — all fall on the wrong side of
   the tradeoff), a sequential scan is faster than paying rayon dispatch cost
   for a `par_chunks` fan-out, *especially* when it's nested inside an
   already-parallel outer loop. Keep the parallel path for the one case it
   clearly wins: a single very large file with no outer parallelism
   competing for the pool (the `bench.rs` scenario in `04`).

   **Implemented**: `Model::build_indices` (`model.rs`) gates its
   `par_chunks` fan-out behind `PARALLEL_INDEX_THRESHOLD` (100,000 cards),
   scanning sequentially below it.
3. **Parallelize `validate()`.** It's the single largest stage in this
   workload (3.68 s, ~34% of wall time) and is plain sequential over
   `self.cells()` / `self.surfaces()` today, despite being read-only and
   embarrassingly parallel per-card — the same map-then-merge shape
   `build_indices`/`renumber_pass`/`reindex` already use (chunk, collect
   `Vec<String>` per chunk, flatten + sort + dedup at the end). Unlike (1)
   and (2), this runs once, after all the small nested dispatches are done,
   on the one genuinely large object (the fully-composed model) — a case
   where full-width parallelism is actually the right call.

   **Implemented**: `compose.rs`'s `validate_chunked` does exactly this
   chunk/flatten shape, gated by `PARALLEL_VALIDATE_THRESHOLD`.
4. **`merge()`'s `by_kind` collection is sequential** card-by-card
   `.to_owned()` cloning across every incoming model (1.86 s here, second
   largest stage). Worth parallelizing the per-model text collection given
   it's the same order of magnitude as `validate()` in this workload.

   **Implemented**: `Model::merge` (`compose.rs`) collects each incoming
   model's cards via `into_par_iter` above `PARALLEL_MERGE_THRESHOLD`.
5. **`clear_data_cards()` forces a full source-text round trip and
   re-parse** (re-lex + a fresh `build_indices`) to drop one block from an
   already-parsed model. `compose.rs` justifies going through `parse` for
   extract/merge-adjacent operations because "these are not on the
   interactive hot path" — true for a single interactive edit, false for a
   caller that calls it once per component of a multi-hundred-file project
   (172× here, all sequential, each paying a full nested-parallel parse).
   Dropping a block is structural (contiguous card range within one
   `CardKind` block); doing it via the existing `Cst` removal primitives
   (the same kind used by `remove_cell_param`) instead of text + reparse
   would skip both the re-lex and the redundant index rebuild.

   **Implemented**: `Model::clear_data_cards` now clones the kept `Card`s
   directly and builds the result via `Cst::from_cards`, paying only an
   index scan — no text round-trip, no re-lex.

## How to verify a fix

Re-run the thread-count sweep above after any of the above changes: the bar
is `sys` time tracking close to `real` time (not growing with thread count)
at the default pool size, with `real` time unchanged or improved. `gitronics`
building `big_example/configurations/baseline.yaml` is the reference
workload; it isn't checked into this repo, but any multi-file project of
comparable shape (hundreds of small-to-medium models composed into one) will
reproduce the pattern — a single huge file (`big.mcnp`, `04`'s benchmark)
will not, since it never exercises the nested outer/inner parallelism this
doc is about.
