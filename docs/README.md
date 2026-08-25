# migjorn — design docs

`migjorn` is a fast, **lossless**, general-purpose MCNP input parser and editor in
Rust with Python bindings. It parses an MCNP input file into a typed, editable
model and re-emits it **byte-for-byte identical except where you changed it**. It
is built for interactive, iterative editing of very large models — opening a
~380 MB, ~1M-card file from Python, exploring it, making many small edits
(add/remove cells & surfaces, renumber, tweak parameters), and writing it back,
all in interactive time.

## The non-negotiable invariants

1. **Lossless**: `parse(t).to_source() == t`, byte for byte — comments, spacing,
   blank-line block delimiters, `&` and blank-column continuations, inline `$`
   comments, UTF-8 BOM, trailing whitespace, line endings. After an edit,
   identical except at edited cards.
2. **Recoverable**: never panics on malformed input; collects diagnostics,
   returns a best-effort model, still round-trips losslessly.
3. **No editing operation re-lexes or re-parses the whole file.** Add/remove a
   card, set a value, renumber — all local.
4. **Consistent reads**: a read after a write sees the write, with no explicit
   flush/materialize step.
5. **Definitions and references renumber together** (surfaces + geometry senses;
   cells + `#n`/`LIKE n`; materials + `Mn`/`MTn`/`MXn`; transforms; universes).

## Read in this order

1. [`01-goals-and-invariants.md`](01-goals-and-invariants.md) — what we build and
   the non-negotiables.
2. [`02-architecture.md`](02-architecture.md) — the core decision (cards as the
   atomic unit, each owning its own text + tokens) and the measured numbers that
   justify it.
3. [`03-mcnp-reference.md`](03-mcnp-reference.md) — the format subset and its
   gotchas.
4. [`04-performance-budget.md`](04-performance-budget.md) — the targets, how to
   measure, the measured real-model results, and the one known limitation
   (parse running 5-8% over budget on high-core-count machines).
5. [`05-parallelism-overhead.md`](05-parallelism-overhead.md) — why more
   threads can make a multi-model workload slower (kernel time, not compute),
   root-caused via profiling `gitronics`, and the required fixes.

## Crate layout

- `migjorn-syntax` — lexer + per-card CST (`Cst`, `Card`, `Token`, `SyntaxKind`);
  losslessness and structural-edit primitives.
- `migjorn` — typed `Model`: projection, editing, renumber, compose, validate.
  The single Rust facade.
- `migjorn-py` — PyO3 + maturin (`abi3`) bindings; the `migjorn` Python module.

## Build & test

```sh
cargo test                 # Rust unit + integration (round-trip, expectations)
cargo clippy               # lint (kept warning-clean)

# Python extension — use a venv scoped to this repo, not a --user install:
# a global install is shared with any other checkout of this project on the
# same machine, so building elsewhere silently shadows this repo's bindings.
python3 -m venv .venv && .venv/bin/pip install pytest
cd crates/migjorn-py && maturin build --release --out ../../dist && cd ../..
.venv/bin/pip install --force-reinstall --no-deps dist/migjorn-*.whl
.venv/bin/pytest tests/ crates/migjorn-py/tests/ -q   # Python acceptance suite +
                                                       # PyO3-boundary tests (exceptions,
                                                       # live-handle semantics, stub drift)
python crates/migjorn-py/examples/showcase.py   # self-contained API tour
```

The round-trip suite is data-driven: drop a `.mcnp` into `tests/corpus/` and it is
auto-held to the losslessness invariant with no extra code.

## Performance

The whole edit/emit/lookup surface meets its budget on the real 377 MB model with
wide margin (see [`04-performance-budget.md`](04-performance-budget.md)). Bench it
with `cargo run --release -p migjorn --example bench -- <input.mcnp>`
(`benches/gen_input.py` produces a reproducible large stand-in). The only row
over budget is raw `parse`, by 5-8% on high-core-count machines; the details
are in `04`.

## Status

All milestones (M0 lossless round-trip → M6 Python bindings) are implemented. The
Rust and Python test suites are green and are the acceptance contract.
