# migjorn rewrite — bootstrap kit

This folder is the **specification and bootstrap kit** for a ground-up
reimplementation of `migjorn`, an MCNP input parser + editor. It exists because
the current engine parses and renumbers large files well but **cannot edit them
cheaply** — a single `add_cell`/`remove_cell` on a 380 MB model costs ~2.4 s
because it re-parses the whole file (see the measured evidence in
[`docs/02-architecture.md`](docs/02-architecture.md)). The target workload —
interactively exploring and modifying huge models from Python — needs a
different core.

Everything here is design + tests + API sketches. **No implementation yet** —
that is the job this kit kicks off.

## Read in this order

1. **[`KICKOFF_PROMPT.md`](KICKOFF_PROMPT.md)** — the prompt to start the build
   with. Self-contained; points at everything below.
2. **[`docs/01-goals-and-invariants.md`](docs/01-goals-and-invariants.md)** —
   what we're building and the non-negotiables (lossless, recoverable, fast for
   editing, consistent reads).
3. **[`docs/02-architecture.md`](docs/02-architecture.md)** — the core decision
   (cards as the atomic unit), why the current design can't be patched into it,
   and the measured numbers that prove it.
4. **[`docs/03-mcnp-reference.md`](docs/03-mcnp-reference.md)** — the format
   subset and its gotchas.
5. **[`docs/04-performance-budget.md`](docs/04-performance-budget.md)** — the
   concrete targets and how to measure them.

## What's here

```
redesign/
├── KICKOFF_PROMPT.md          # paste-to-start prompt for the new build
├── README.md                  # this file
├── docs/
│   ├── 01-goals-and-invariants.md
│   ├── 02-architecture.md     # THE design; measured current-vs-target numbers
│   ├── 03-mcnp-reference.md   # format subset + gotchas
│   └── 04-performance-budget.md
├── api/
│   ├── migjorn.pyi            # target Python API (sketch)
│   ├── rust_api.rs            # target Rust API + core data model (sketch)
│   └── session_example.py     # worked iterative-editing session
├── tests/
│   ├── corpus/*.mcnp          # fixtures — lossless round-trip harness
│   ├── expectations.toml      # pinned parse facts per fixture
│   ├── test_roundtrip.py      # byte-for-byte round-trip (all fixtures, no config)
│   ├── test_parsing.py        # asserts expectations.toml via the API
│   └── test_editing.py        # editing semantics + no-reparse guarantees
└── benches/
    ├── gen_input.py           # synthesize a large model (reproducible perf input)
    └── bench_plan.md          # what to bench + thresholds
```

## The tests are the acceptance contract

The `tests/` here are written against the API in `api/migjorn.pyi`; they don't
run until the implementation exists, but they *are* the definition of done:

- **`test_roundtrip.py`** — the losslessness invariant, auto-applied to every
  fixture. Drop a new `.mcnp` in `corpus/` and it is held to the invariant with
  no extra code.
- **`test_parsing.py`** — correct structural parsing, pinned in
  `expectations.toml`.
- **`test_editing.py`** — the editing semantics the rewrite is *for*.

When the new project stands up, copy `tests/` and `benches/` into it (or point
its CI at this folder) and make them green.

## Status

Design complete; implementation not started. Start with `KICKOFF_PROMPT.md`.
