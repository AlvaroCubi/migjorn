# Kickoff prompt — migjorn rewrite

> Paste this as the opening instruction for the new project. It is self-contained
> but references the design docs and tests in this `redesign/` folder, which
> should be copied into (or alongside) the new repo.

---

## Mission

Build `migjorn`: a fast, **lossless**, general-purpose MCNP input parser and
editor in Rust with Python bindings. It parses an MCNP input file into a typed,
editable model and re-emits it **byte-for-byte identical except where the user
changed it**. It is optimized for **interactive, iterative editing of very large
models** — a Python/notebook session opening a ~380 MB, ~1M-card file, exploring
it, making many small edits (add/remove cells & surfaces, renumber, tweak
parameters), and writing it back, all in interactive time.

This is a ground-up build. A previous implementation exists and works, but its
core (a single flat token arena spanning the whole file) makes structural edits
cost a full-file reparse (~2.4 s per `add_cell` on the big model). We are
replacing that core. Reuse the previous project's **domain logic and public API
shape** (they are good — see `api/` and `docs/03`), not its core data structure.

## The non-negotiable invariants

1. **Lossless**: `parse(t).to_source() == t`, byte for byte — comments, spacing,
   blank-line block delimiters, `&` and blank-column continuations, inline `$`
   comments, UTF-8 BOM, trailing whitespace, line endings. After an edit,
   identical except at edited cards.
2. **Recoverable**: never panic on malformed input; collect diagnostics, return
   a best-effort model, still round-trip losslessly. (Release profile must NOT
   set `panic = "abort"` — PyO3 needs unwinding.)
3. **No editing operation re-lexes or re-parses the whole file.** Add/remove a
   card, set a value, renumber — all local. This is the entire reason for the
   rewrite; treat any whole-file reparse on an edit path as a design failure.
4. **Consistent reads**: a read after a write sees the write, with no explicit
   flush/materialize step, and no reader can observe state disagreeing with
   `to_source()`.
5. **Definitions and references renumber together** (surfaces + geometry senses;
   cells + `#n`/`LIKE n`/tally bins; materials, transforms, universes, tallies).

## The architecture (already decided — see `docs/02-architecture.md`)

**Cards are the atomic unit.** The model is a `Vec<Card>`; each card owns (or,
while pristine, borrows) its own small token buffer.

- `Cst { source: Arc<str>, arena: Arc<[Token]>, cards: Vec<Card> }`.
- `Card` text is a **hybrid**: `Pristine { span, tokens-range }` (borrows the
  shared source + token arena, zero per-card allocation — so parse stays ~1 s)
  or `Owned { String, Vec<Token> }` (only after the card is edited or newly
  built).
- Structural edits mutate **only** `cards` (a `Vec` insert/remove); `source` and
  `arena` are never mutated. Add = lex the one snippet (µs) + `Vec::insert`.
  Remove = `Vec::remove`.
- In-card edits set a local token override, promoting that card to `Owned`.
- Emit walks `cards`: pristine → memcpy `source[span]`; owned → render tokens.
  Untouched cards stay a memcpy, so emission never regresses to O(all tokens).
- Stable per-card `slot` ids; live handles resolve slot → index each use, so an
  edit to one card never invalidates a handle to another.
- Maintained `id → slot` indices for O(1) lookup.
- Cards are independent ⇒ parse, renumber, and emit are data-parallel with
  `rayon` for the stretch targets.

This deletes the previous design's `OwnedCell`/promotion, emit-only splices,
`has_pending_splices`/`materialize()`, and the `Model`/`ModelView` split — none
are needed when the card list is always the current state.

See `api/rust_api.rs` for the concrete type/signature sketch to build against.

## Crate layout

- `migjorn-syntax` — lexer + per-card CST (`Cst`, `Card`, `Token`, `SyntaxKind`);
  losslessness and structural-edit primitives.
- `migjorn` — typed `Model`: projection, editing, renumber, compose, validate.
  The single public facade.
- `migjorn-py` — PyO3 + maturin `abi3` bindings mirroring `api/migjorn.pyi`.

## Performance budget (full table in `docs/04-performance-budget.md`)

On a ~1M-card model, release build:

| Operation | Target |
|---|---|
| parse | ≤ 1.0 s (stretch ≤ 0.5 s parallel) |
| emit unedited / after K edits | ≤ 0.20 s / ≤ 0.20 s + O(K) |
| single add_cell / remove_cell | ≤ 10 ms |
| 1000-edit session | ≤ 1.0 s total |
| read after edit | ≤ 1 ms |
| in-card value edit | ≤ 5 µs |
| renumber_cells / renumber_surfaces (incl. emit) | ≤ 0.5 s / ≤ 2.0 s |
| id lookup | O(1) |

`benches/gen_input.py` produces a reproducible large input; `benches/bench_plan.md`
says how to measure and how to fail CI on a regression.

## Definition of done

1. `tests/test_roundtrip.py` green — every corpus fixture round-trips byte-for-
   byte, malformed input never raises.
2. `tests/test_parsing.py` green — every fact in `tests/expectations.toml` holds.
3. `tests/test_editing.py` green — editing semantics + no-reparse behavior.
4. `benches/` prints the budget table with every row passing on the ~1M-card
   input.
5. `api/session_example.py` runs in interactive time.

## Suggested milestones (build the invariant-critical path first)

- **M0 — Lexer + lossless round-trip.** `Cst::parse` + `to_source` on all corpus
  fixtures byte-for-byte, including BOM/whitespace/continuations/malformed. No
  typed layer yet. Gate: `test_roundtrip.py`. This locks the hardest invariant
  before any feature exists.
- **M1 — Typed projection (read-only).** `Model`, `Cell`/`Surface`/`Material`/
  `Transform` views, id indices, lookups. Gate: `test_parsing.py`.
- **M2 — In-card value edits.** set material/density/coeff/param via local token
  overrides; emit stays memcpy for untouched cards. Gate: relevant
  `test_editing.py` + emit-after-K-edits bench.
- **M3 — Structural edits.** add/remove card as `Vec<Card>` ops; stable slots;
  handle invalidation. Gate: structural `test_editing.py` + add/remove/session
  benches (the headline win).
- **M4 — Renumbering.** whole-model defs+refs for cells/surfaces/materials/
  transforms/universes/tallies. Gate: renumber tests + benches.
- **M5 — Composition.** validate / extract_universe / extract_level0 / merge.
- **M6 — Python bindings + parallelism.** PyO3 `abi3` to `api/migjorn.pyi`;
  rayon for parse/renumber/emit to reach stretch targets; stub-drift test.

At each milestone, run the round-trip suite (it must never regress) and the
relevant bench rows.

## Ground rules carried over from the current project

- Corpus tests are data-driven: drop a `.mcnp` in `corpus/`, it's auto-asserted
  lossless — no test code. Keep this; it's the best regression net.
- Keep definitions-and-references-together as a correctness property of every
  renumber pass; the reference-scanning side is as important as the definition
  side.
- Preserve the user's formatting; never reflow a card the user didn't touch.
- After changing the PyO3 surface, update `api/migjorn.pyi` and keep a
  stub-drift test.
