# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`migjorn` is a fast, **lossless**, general-purpose MCNP input parser in Rust with Python bindings. The defining invariant: parse → edit → re-emit reproduces the input byte-for-byte *except where you changed it* — comments, spacing, and continuations are all preserved. Every feature must uphold this. The parser is also **recoverable**: it never panics on malformed input; it collects `Diagnostic`s and returns a best-effort model.

## Commands

Rust (workspace root):
```bash
cargo test --workspace --exclude migjorn-py   # unit + corpus snapshot tests (CI form)
cargo test -p migjorn cell               # single crate / filter by test name
cargo fmt --all                               # CI runs --check
cargo clippy --workspace --exclude migjorn-py --all-targets -- -D warnings
cargo insta review                            # review pending snapshot changes (see Corpus tests)
```
`migjorn-py` is excluded from the Rust jobs because it is a Python extension needing a Python toolchain; it is built and linted in the Python job.

Python (`crates/migjorn-py`, uses maturin + a persistent venv):
```bash
cd crates/migjorn-py && maturin develop --release   # rebuild into your active venv — do this before every pytest run
pytest crates/migjorn-py/tests -q                    # `pytest` alone won't see new Rust code, only a fresh maturin develop does
uvx ruff@0.15.21 check crates/migjorn-py             # lint (CI-pinned version)
uvx ruff@0.15.21 format crates/migjorn-py            # CI runs --check
```
CI instead uses `uv run --no-project --with ./crates/migjorn-py --with pytest pytest ...` (see `ci.yml`) — that ephemeral form is fine there because every run starts from an empty cache, but see the gotcha below for why it's the wrong choice locally.

## Architecture

Three crates, layered bottom-up (each depends only on those below):

- **`migjorn-syntax`** — the lossless syntax layer. Owns the lexer and the concrete syntax tree. The production tree is `GreenTree` (`cst.rs`), a **custom flat arena**, *not* rowan: MCNP tokens tile the source exactly, so tokens are stored as parallel `Vec`s of `(kind, start)` and text is recovered as spans into the original source — no per-token interning or heap traffic. This is the crate's core performance decision (rowan's `GreenNodeBuilder` cost ~10x the lexer; see `docs/m0-findings.md`, which documents the since-removed rowan spike). `GreenTree` also carries an **integer-override overlay** so edits rewrite only touched tokens and re-emission stays a single linear pass.

- **`migjorn`** — the typed AST projected *on demand* over the flat CST. Typed views (`Surface`, `Cell`, `Transform`, `Material`, `DataCard`) carry the token indices they were built from, so edits rewrite the exact tokens while everything else stays byte-for-byte. **`Model` (`model.rs`) is the single public facade** — typed-projection functions, the emitter, numeric parsing, and renumbering internals are deliberately *not* re-exported; all capability is reached through `Model`/`ModelView`. Structurally-edited cells are stored as `OwnedCell` in `Model.owned_cells`, keyed by a monotonic slot that survives reparses so handles stay stable across add/remove.

  **The read/write split.** `Model` owns the edits; **every read goes through `ModelView`, obtained from `Model::view()`**. Some edits are *splices* (token insert/delete, whole-card replacement) that the CST applies only at emit time, so a raw-token reader would disagree with `to_source()`. `Model::materialize` bakes them in by reparsing, and is **lazy** — it early-returns unless `GreenTree::has_pending_splices()`, so N edits cost one reparse at the next read and the hot `parse → renumber → emit` path never reparses (renumbering writes token *overrides* only, which emission already applies; `renumber_uses_overrides_only_so_it_never_forces_a_reparse` guards this — keep it passing). `view()` pays the single `&mut` and materializes, so `ModelView`'s readers are all `&self`: they compose, they can be shared, and no reader can observe a tree that disagrees with emission. When adding a reader, put it on `ModelView`, not `Model`. After `materialize()`, `owned_cells` is empty and the CST is the sole source of truth — every `promote_cell` site is paired with a splice, which is what makes that invariant hold (`view()` `debug_assert`s it).

- **`migjorn-py`** — PyO3 + maturin bindings, built as `abi3` wheels (one wheel serves CPython 3.9+). The compiled extension is the private `migjorn._migjorn` submodule; the public `migjorn` package lives in `python/migjorn` and re-exports it plus PEP 561 type stubs.

### Renumbering (the flagship editing capability)
`renumber.rs` must update **definitions *and* every reference** consistently. Renumbering surfaces rewrites each surface card's id *and* every signed surface in cell geometry (preserving sense and `+`/white-boundary prefixes); renumbering cells rewrites ids *and* every `#n` complement and `LIKE n` base reference. Similar whole-model passes exist for materials, transforms, universes, and tallies. All edits go through the CST integer-override overlay. When touching any renumber pass, the reference-scanning side (`scan_cell_refs`, cell-parameter parsing) is as important as the definition side.

## Corpus / snapshot tests

Regression testing is data-driven: drop any `.mcnp` file (or a one-card snippet) into `crates/migjorn-syntax/tests/corpus/` and run `cargo insta review`. It is automatically asserted lossless and snapshotted — **no test code required**. This is the primary way to lock in parser behavior against real inputs.

## Conventions & gotchas

- **No `panic = "abort"`** in the release profile — the Python extension needs panics to unwind so PyO3 converts them to Python exceptions instead of aborting the host process. Don't add it.
- **Stub drift is enforced.** `crates/migjorn-py/tests/test_stubs.py` fails if `python/migjorn/__init__.pyi` and the runtime API disagree. After changing the PyO3 surface, update the `.pyi` stubs.
- **An editor that decides from the card's current tokens must `materialize()` first.** Splices are emit-only, so a raw-token read cannot see a previous one and the edit silently does the wrong thing (e.g. `set_surface_transform(Some(1))` then `(None)` failing to remove it). Any read in between hides the bug, which is why it evaded the tests for so long — `consecutive_edits_do_not_read_stale_tokens` locks the cases down. The `owned_cells`-backed geometry editors (`add_cell_surface`, `remove_cell_complement`, …) are the exception: `owned_cells` is their source of truth, so they stay materialize-free and keep their edit batching.
- **The Python extension can silently be stale — this is why local testing uses a persistent venv, not `uv run --with`.** `maturin develop --release` shells out to `cargo build` directly, so it always sees edits to the Rust core (`crates/migjorn/`); `uv run --with ./crates/migjorn-py`, by contrast, keys its build cache on the `migjorn-py` directory alone and will happily keep testing old code after a core-only edit, and neither `--refresh` nor `uv cache clean migjorn` fixes it (force it with `rm -rf ~/.cache/uv/archive-v0 ~/.cache/uv/sdists-v9` if you do end up needing that ephemeral form, e.g. for the notebook workflow in `crates/migjorn-py/README.md`). Also delete any stale `crates/migjorn-py/python/migjorn/_migjorn.abi3.so` left by an earlier `maturin develop`: it sits in the packaged `python-source` dir and shadows fresh builds. When a Python result contradicts a Rust test, suspect one of these first.
- Design notes and benchmarks live in `docs/` (`m0`…`m9`), each documenting a milestone (findings, renumber, editing, structural editing, construction, renumber generalization).
