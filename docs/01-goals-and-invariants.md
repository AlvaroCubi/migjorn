# 01 — Goals & Invariants

## What we are building

A fast, **lossless**, general-purpose MCNP input parser in Rust with Python
bindings, designed from the start for **interactive, iterative editing of very
large models** (hundreds of MB, ~1M cards). The user opens a model in a Python
session (or notebook), explores it, makes many small edits — add/remove cells
and surfaces, renumber, tweak parameters — reads the results back, and writes it
out, all in interactive time.

This is a **ground-up reimplementation**, not a refactor. It inherits the public
API shape and the domain knowledge of the current library (both are good but feel 
free to improve them or change them if needed), but replaces the core data structure, 
which was built for a parse-renumber-emit batch pipeline and does not support cheap
structural editing (see [`02-architecture.md`](02-architecture.md) for the measured
evidence).

## The invariants — every feature must uphold these

### 1. Lossless
`parse(text).to_source() == text`, **byte for byte** — including comments,
spacing, blank-line delimiters, continuations (`&` and blank-column), inline
`$` comments, a UTF-8 BOM, trailing whitespace, and the original line endings.
After an edit, the output is identical *except at the cards you changed*.

This is the single most important promise. It is checked for every corpus file,
with no per-file configuration, by `tests/test_roundtrip.py`. A change that
breaks it is wrong by definition.

### 2. Recoverable
The parser **never panics** on malformed input. It classifies what it can,
records `Diagnostic`s for what it can't, and returns a best-effort model. A
malformed card still round-trips losslessly (we preserve bytes we don't
understand). `tests/corpus/malformed_recoverable.mcnp` exercises this.

> Rust note: the release profile must **not** set `panic = "abort"` — the Python
> extension needs panics to unwind so PyO3 converts them to exceptions instead
> of aborting the host interpreter.

### 3. Fast — and fast *specifically for iterative editing*
Parsing a ~380 MB model stays around 1 second, and — the point of the
rewrite — **no editing operation triggers a whole-file reparse or re-lex.**
Add/remove a card, set a value, renumber: all local, all cheap. Concrete
targets and thresholds are in [`04-performance-budget.md`](04-performance-budget.md)
and enforced by `benches/`.

### 4. Consistent reads
A read after a write on the same logical object observes the write, with no
explicit "flush"/"materialize" step. There is no mode in which a reader can
observe state that disagrees with `to_source()`. (In the current design this
required a hand-maintained `materialize()` discipline guarded by `debug_assert`s;
the new design makes it structural — the card list *is* the state.)

### 5. Definitions and references move together
Renumbering is a whole-model operation: renumbering surfaces rewrites every
surface card's id **and** every signed surface in cell geometry (sense and
`+`/white prefixes preserved); renumbering cells rewrites ids **and** every `#n`
complement and `LIKE n` base. The same holds for materials, transforms and
universes. A renumber that updates definitions but misses a reference is a
correctness bug, not a cosmetic one.

Tallies are the one family where this does not (yet) extend to references:
`renumber_tallies` moves each tally card's own id, but a cell/surface number
used as a tally *bin* (`f4:n 1 2 3`) is not currently rewritten by
`renumber_cells`/`renumber_surfaces`. See `03-mcnp-reference.md`.

## Non-goals

- **Not** a physics/transport engine — we parse and edit input, we don't run it.
- **Not** a semantic validator beyond the cheap consistency checks `validate()`
  already offers (dangling references, duplicate ids, undefined transforms).
- **Not** a normalizer/pretty-printer — we preserve the user's formatting; we
  never reflow or "clean up" a card the user didn't touch.
- Full modelling of every exotic data card's grammar is **out of scope** at the moment: data
  cards we don't specifically type are still lexed, round-tripped, and
  renumber-scanned generically. We type the cards the API exposes (cells,
  surfaces, `Mn`, `TRn`, tallies, `fill`/`u`/`trcl`); the rest pass through.

## Definition of done

- Every file in `tests/corpus/` round-trips byte-for-byte (`test_roundtrip.py`).
- Every pinned fact in `tests/expectations.toml` holds (`test_parsing.py`).
- Every editing-semantics test passes (`test_editing.py`).
- The performance budget in `04-performance-budget.md` is met on a large input
  (`benches/`).
- The Python API matches `crates/migjorn-py/migjorn.pyi` and the worked
  session in `crates/migjorn-py/examples/showcase.py` runs in interactive
  time.
