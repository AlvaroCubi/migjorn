# 02 — Architecture

## The one decision that drives everything: **cards are the atomic unit**

The model is an ordered `Vec<Card>`, where **each card uniformly owns its own
text (`String`) and token buffer (`Vec<Token>`)**. Every card is the same shape
and is handled the same way regardless of whether or how it has been edited —
there is no borrowed/owned split and no edit overlay. This is the whole design.
Everything else follows:

| Operation | Cost | Why |
|---|---|---|
| Parse | O(bytes), parallelizable per block | lex each card, copy its bytes + build its tokens |
| Emit (`to_source`) | O(bytes) | concatenate every card's `text` — a memcpy per card, edited or not |
| Add / remove a card | O(num_cards) memmove of `Card` structs | it's a `Vec` insert/remove; the snippet is lexed alone (µs) |
| In-card value edit / renumber a token | O(card length) | splice the new text into that one card's buffer |
| Read a cell/surface | O(card length) on-demand projection | typed view built from that card's tokens |
| Lookup by id | O(1) | maintained `id → slot` index |

Crucially, **no editing operation re-lexes the whole file.** That single
property is the reason for the rewrite. A card is small (tens of tokens, tens to
hundreds of bytes), so "O(card length)" edits are microseconds.

## Why not just optimize the current design?

The current core is one **flat token arena spanning the entire file** (`GreenTree`:
parallel `Vec`s of `(kind, start)`, text recovered as spans into the original
source). That was a deliberate, well-reasoned choice for a batch
*parse → renumber → emit* pipeline, and it is genuinely excellent at it. But it
makes the file one indivisible unit, and that is fatal for interactive editing.
Measured on `big.mcnp` (377 MB, ~1.02M cards, 425k cells), release build:

| Operation | Current cost | Root cause |
|---|---|---|
| parse | **0.96 s** ✅ | (fine — the flat arena is fast to build) |
| emit, unedited | **0.16 s** ✅ | memcpy fast path |
| **single `add_cell`** | **2.42 s** ❌ | `to_source()` the whole file, then **re-`parse()` the whole file** |
| **single `remove_cell`** | **2.44 s** ❌ | same |
| **10 adds in a loop** | **24.4 s** (2.44 s each, no amortization) ❌ | each pays the full reparse |
| `add_cell_param` (the splice) | 5 µs ✅ | splice is emit-only... |
| **`view()` after that splice** | **1.65 s** ❌ | ...but the next *read* must `materialize()` = reparse the file |
| renumber_cells | 0.64 s scan + 1.71 s emit ❌ | any override disables the whole-file emit fast path |
| renumber_surfaces (24.3M refs) | 2.56 s scan + 5.78 s emit ❌ | 24M overrides + no-fast-path emit |

The 2.4 s figures are not a slow path to tune — they are structural. Because
card boundaries change token indices across the whole arena, the only correct
way the current design can absorb a structural edit is to re-emit and re-lex
everything (`model.rs::reparse_with_change`, `materialize`). No amount of
micro-optimization removes an O(file) reparse from a single-card edit.

The emission cliff is the same story: `to_source()` has a fast path only when
there are **zero** overrides/splices anywhere (`cst.rs::to_source`); one edit
anywhere disables it for the *entire* file, and the fallback then does 2–3 hash
lookups per token over ~100M tokens.

Both problems dissolve when the card, not the file, is the unit: an edit is
local, and emission concatenates self-contained cards.

## The data model

```
Model
 ├─ cst: Cst
 │    └─ cards: Vec<Card>            // self-contained; the whole tree
 ├─ diagnostics: Vec<Diagnostic>
 └─ cell_index / surface_index / material_index / transform_index : id → slot
```

There is no shared source buffer and no global token arena. Each card is
self-contained, so the only shared structure is the `Vec<Card>` itself, and the
original source is not retained after parse.

### `Card`
```
Card {
  slot:   u32,
  kind:   CardKind,
  text:   String,        // this card's exact current bytes
  tokens: Vec<Token>,    // tokens tiling `text` (spans relative to `text`)
}
```

**Uniform ownership is the whole simplification.** Every card — freshly parsed,
never touched, or edited a hundred times — is the same: a `String` of its exact
current bytes and the `Vec<Token>` that tiles it. Consequences:

- **Emit** is `for card in cards { out.push_str(&card.text) }`. An *unedited*
  card's `text` already holds its original bytes, so this is a memcpy per card
  whether or not the card was edited — there is no fast/slow path to branch on,
  and byte-for-byte losslessness is automatic.
- **Any in-card edit** (renumber a token, set a value, add/remove a parameter,
  splice a surface) mutates that one card's `text` and `tokens` in place. Same
  cost class, same code path, no mode change.
- **Reads** project a typed view from the card's own token slice.

### Why uniform, and not a shared arena with borrowed cards

An alternative keeps one immutable `source` + token arena and lets pristine cards
*borrow* into them, copying a card to owned only on edit. It is faster to parse —
but **only** to parse. Measured over `big.mcnp` (377 MB, 116.6M tokens, ~928k
cards), the shared-arena vs. all-owned difference is:

| | shared-arena (borrow) | all-owned (this design) |
|---|---:|---:|
| build cards (beyond lexing) | ~6 ms | **~940 ms** |
| emit | 177 ms | 194 ms (≈ same) |
| full renumber (rebuild touched card strings) | — | ~800 ms |
| steady-state memory | ~1.8 GB | ~1.8 GB (≈ same) |

The all-owned penalty is entirely **~0.9 s of per-card copying at parse** (~2M
allocations: one `String` + one `Vec<Token>` per card). Everything else is a
wash: emit is identical (an unedited owned card is already a memcpy), and
steady-state memory is comparable because ~1.4 GB of *tokens* dominate and exist
in both designs — all-owned adds only per-card header overhead, not a second copy
of the file.

Critically, the shared-arena optimization makes **no edit faster**: a single edit
rebuilds only its one small card either way, in microseconds. It buys ~0.9 s of
one-time parse latency (which parallel per-card construction largely reclaims —
see Parallelism) in exchange for a `CardText` enum, an overrides overlay, a
retained source/arena, and "is this card a memcpy" branching everywhere. For a
library whose whole purpose is *simple, fast iterative editing*, that is a bad
trade. **We take the uniform design; the shared arena is a documented, deferrable
parse-latency optimization to reach for only if a profile demands it.**

### Stable slots
Each card carries a `slot: u32` that is unique for the model's lifetime and
never reused. A live handle (Python `Cell`, Rust `CellView`) stores a slot, and
resolves it to a current index via `card_of_slot` on each use. Therefore an edit
to card *A* never invalidates a handle to card *B* — the only way a handle goes
bad is if *its own* card is removed, which is detected and raised cleanly. This
replaces the current library's `owned_cells`/promotion/`materialize` machinery
entirely.

### Tokens tile the source (kept from the current design)
Within a card, tokens — **including trivia** (whitespace, newlines, comments,
`&`) — cover the card's text with no gaps. This is what makes losslessness a
concatenation and text access a span, with no interning. We keep this; we just
scope it to a card instead of the file.

## How each capability maps on

- **Emit**: `for card in cards { out.push_str(&card.text) }`. O(bytes),
  byte-identical when unedited, one uniform path.
- **add_cell(text)**: validate it's one card of the right kind, `lex_card` it
  (µs), find the insert position (end of the matching block), `cards.insert`,
  assign a fresh slot, update the id index. No reparse.
- **remove_cell(id)**: index → slot → position, `cards.remove`, drop from index.
  (Also removes the card's trailing blank/continuation lines as their own cards
  if they belong to it — layout preserved.)
- **set value / renumber a token**: `set_token_text(card, token, ...)` splices
  the new text into that card's `text` and shifts the following token spans.
  O(card length). No other card is touched.
- **renumber_* (whole model)**: iterate `cards` once, scan each cell/surface's
  tokens for definitions and references, and rewrite the matching tokens in each
  touched card's own buffer. Parallelizable (see below).
- **read (typed projection)**: build a `CellView`/`SurfaceView` on demand from
  the card's tokens — same projection logic as today, now over a card-local
  token slice. Reads are `&self`; writes are addressed by slot through `Model`,
  so two cells can be read at once without borrow conflicts.
- **validate / extract_universe / merge**: whole-model passes over `cards`,
  building new `Cst`s where needed. `merge`/`extract` construct a new model from
  selected cards — a `Vec<Card>` splice, not string surgery.

### A note on the cost of a *full* renumber

Renumber is the one operation that touches (nearly) every card, so it is
inherently the heaviest edit. What matters is that it stays *proportional to the
work*, with a low constant, and parallelizes. On `big.mcnp`, renumbering all
surfaces is **24.3M** token rewrites (every signed surface in every cell + ~590k
defs); renumbering all cells is 427k. That reference count is an irreducible
floor — no data structure makes it free.

The design keeps the constant low in two ways the current engine can't:
1. **No global override map.** Each rewrite mutates the token's card-local
   buffer directly — there is no one `FxHashMap` with tens of millions of
   entries that emit must then probe per token. This is the biggest win: the
   current engine's 5.78 s surface-renumber *emit* is dominated by that giant
   map, whereas here emit is always the same plain per-card memcpy (~190 ms).
2. **Embarrassingly parallel.** Cards are independent, so both the scan/rewrite
   and the emit are `par_iter` across cards; the `id → slot` map is read-only
   during the scan.

Rebuilding the touched cards' strings costs ~0.8 s single-threaded (measured),
comparable to the current engine's scan and far below its emit. Net expectation
vs. the current 8.3 s (surfaces) / 2.3 s (cells): comparable single-threaded
after removing the global-map overhead, and **≤ 2 s / ≤ 0.5 s** (the budget
targets) with modest parallelism. So: not "very expensive", but honestly the
*least improved* operation — because it is the one that genuinely must look at
the whole model.

## Parallelism (how we reach the stretch targets)
Cards are independent, which makes the expensive whole-model passes data-parallel
with `rayon`:
- **Parse**: split source at block/blank boundaries, then lex-and-build cards in
  parallel. This matters most here: the per-card copy (String + tokens) is ~0.9 s
  of the parse cost single-threaded, and it is exactly the parallelizable part —
  ~0.9 s across 8 cores is ~0.15 s, which is what keeps all-owned parse in the
  ~1 s budget despite the upfront copy.
- **Renumber**: the per-card scan/rewrite is `par_iter`; the `id → slot` map is
  read-only during the scan.
- **Emit**: render cards in parallel into per-card buffers, then concatenate; or
  chunk the card list and emit chunks in parallel.

Single-threaded should meet the base budget for everything except parse, where
the per-card copy makes parallel construction the intended path to ≤ 1 s.

## Crate layout (unchanged — it's good)
- **`migjorn-syntax`** — lexer + the per-card CST (`Cst`, `Card`, `Token`,
  `SyntaxKind`). Owns losslessness and structural editing primitives.
- **`migjorn`** — the typed `Model`: projection, editing, renumber, compose,
  validate. The single public facade.
- **`migjorn-py`** — PyO3 + maturin `abi3` bindings mirroring `api/migjorn.pyi`.

## What this design *deletes*
Complexity that existed only to work around the flat arena's inability to edit
structurally — and, now, the complexity a shared-arena rewrite would itself add:
- `OwnedCell` + `promote_cell` + the monotonic-slot promotion bookkeeping.
- Emit-only splices + `has_pending_splices` + lazy `materialize()` + the
  `debug_assert` invariant that "the CST is the sole source of truth after
  materialize."
- The `Model` / `ModelView` read-write split that existed to force a
  `materialize()` before any read.
- A `Pristine`/`Owned` card enum, a per-card overrides overlay, a retained
  shared `source`/`arena`, and every "is this card a memcpy" branch — none of
  which exist in the uniform design.

With cards as the unit and every card self-contained, the card list is always the
current, consistent state. There is nothing to materialize and nothing to
promote.

## Risks & mitigations
- **Parse pays a ~0.9 s per-card copy (single-threaded).** This is the price of
  uniform ownership. Mitigation: parallel per-card construction (above) brings it
  to ~0.15 s; measured, it is the parallelizable part of parse. If parse latency
  ever becomes the bottleneck even so, the shared-arena/borrow scheme is a
  localized optimization behind the `Card` API — add it then, profile-driven,
  without touching edit logic.
- **Per-card memmove on insert/remove is O(num_cards).** For ~1M cards a memmove
  of `Card` structs (a `String` + `Vec` header + slot/kind ≈ 56 bytes each) is
  single-digit milliseconds — fine for interactive edits, and far below the
  current 2.4 s. If a workload does *millions* of structural edits, offer a batch
  API (`add_cells(iter)`) that does one memmove. (Do not prematurely switch to a
  gap buffer / linked structure; measure first.)
- **Per-card allocation overhead.** ~2M small allocations at parse and ~tens of
  MB of `String`/`Vec` headers. Bounded and comparable to the arena's own
  footprint (tokens dominate either way); a bump/arena allocator for the token
  vecs is an option if allocation pressure shows up in a profile.
