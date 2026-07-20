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

**Cards are the atomic unit, and every card is uniform.** The model is a
`Vec<Card>`; each card owns its own text (`String`) and token buffer
(`Vec<Token>`) — the same shape whether it was just parsed, never touched, or
edited a hundred times. No borrowed/owned split, no edit overlay.

- `Cst { cards: Vec<Card> }`. `Card { slot: u32, kind, text: String,
  tokens: Vec<Token> }` where token spans are relative to the card's `text`.
  There is no shared source buffer or token arena, and the source is not retained
  after parse.
- Structural edits mutate **only** `cards` (a `Vec` insert/remove). Add = lex the
  one snippet into a self-contained `Card` (µs) + `Vec::insert`. Remove =
  `Vec::remove`.
- In-card edits (renumber a token, set a value, add/remove a parameter) splice
  that one card's `text` and fix its token spans. Same code path for all of them;
  no mode change, nothing to "promote".
- Emit is `for card in cards { out.push_str(&card.text) }` — a memcpy per card,
  edited or not (an unedited card's `text` already holds its exact bytes). One
  uniform path; byte-for-byte lossless when unedited.
- Stable per-card `slot` ids; live handles resolve slot → index each use, so an
  edit to one card never invalidates a handle to another.
- Maintained `id → slot` indices for O(1) lookup.
- Cards are independent ⇒ parse, renumber, and emit are data-parallel with
  `rayon`. This matters most at **parse**: giving every card its own copy costs
  ~0.9 s single-threaded on the big model, and parallel per-card construction is
  what keeps parse in the ≤ 1 s budget.

**Why uniform and not a shared arena with borrowed cards?** Measured: the
shared-arena/borrow scheme saves ~0.9 s of *parse* time and nothing else — emit
is identical, memory is comparable, and it makes no *edit* faster (a single edit
rebuilds only its one small card either way). It would cost a `CardText` enum, an
overrides overlay, a retained source/arena, and "is this card a memcpy" branching
throughout. For a library about simple, fast iterative editing that is a bad
trade. Take the uniform design; treat the shared arena as a documented,
deferrable parse-latency optimization to add only if a profile demands it.

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
| parse | ≤ 1.0 s (needs parallel per-card build) |
| emit unedited / after K edits | ≤ 0.20 s / ≤ 0.20 s (same uniform path) |
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
- **M2 — In-card value edits.** set material/density/coeff/param by splicing the
  one card's `text` + tokens. Gate: relevant `test_editing.py` + emit-after-K-
  edits bench (must stay flat — emit cost is edit-independent).
- **M3 — Structural edits.** add/remove card as `Vec<Card>` ops; stable slots;
  handle invalidation. Gate: structural `test_editing.py` + add/remove/session
  benches (the headline win).
- **M4 — Renumbering.** whole-model defs+refs for cells/surfaces/materials/
  transforms/universes/tallies. Gate: renumber tests + benches.
- **M5 — Composition.** validate / extract_universe / extract_level0 / merge.
- **M6 — Python bindings + parallelism.** PyO3 `abi3` to `api/migjorn.pyi`;
  rayon for parse (per-card build)/renumber/emit to hit the budget; stub-drift
  test.

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
