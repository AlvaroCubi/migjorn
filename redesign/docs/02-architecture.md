# 02 — Architecture

## The one decision that drives everything: **cards are the atomic unit**

The model is an ordered `Vec<Card>`, where **each card owns (or, while pristine,
borrows) its own small token buffer**. This is the whole design. Everything else
follows:

| Operation | Cost | Why |
|---|---|---|
| Parse | O(bytes), parallelizable per block | cards are independent; lex each, collect |
| Emit (`to_source`) | O(bytes) + O(edited cards) | untouched cards are memcpy'd from source; only edited cards render |
| Add / remove a card | O(num_cards) memmove of light structs | it's a `Vec` insert/remove; the snippet is lexed alone (µs) |
| In-card value edit / renumber a token | O(1) local override | touches only that card's tokens |
| Read a cell/surface | O(card length) on-demand projection | typed view built from that card's tokens |
| Lookup by id | O(1) | maintained `id → slot` index |

Crucially, **no editing operation re-lexes the whole file.** That single
property is the reason for the rewrite.

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
local, and emission copies the untouched 99.99% verbatim.

## The data model

```
Model
 ├─ cst: Cst
 │    ├─ source: Arc<str>            // immutable snapshot of the original bytes
 │    ├─ arena:  Arc<[Token]>        // immutable pristine token buffer
 │    └─ cards:  Vec<Card>           // the ONLY thing structural edits mutate
 ├─ diagnostics: Vec<Diagnostic>
 └─ cell_index / surface_index / material_index / transform_index : id → slot
```

### `Card`
```
Card { slot: u32, kind: CardKind, text: CardText }

CardText =
  | Pristine { span: Range<usize>,  tokens: Range<usize> }   // borrows source+arena
  | Owned    { text: String,        tokens: Vec<Token> }     // self-contained
```

**The hybrid is the key trick.** A freshly parsed model is *all* `Pristine`
cards: no per-card allocation, text is `source[span]`, tokens are `arena[range]`
— identical memory profile to today's flat arena, so parse stays ~1 s. A card
becomes `Owned` only the first time it is edited or when newly constructed. So:

- Parse: cheap, no per-card heap traffic.
- Untouched cards after any number of edits: still zero-copy, still memcpy on
  emit.
- Edited/new cards: own a small `String` + `Vec<Token>` (tens of tokens) — the
  cost is proportional to *what you edited*, nothing more.

`source` and `arena` are **never mutated**. Removing a card drops its entry from
`cards`; its bytes linger in the immutable snapshot (unreferenced, harmless).
`Cst::compact()` can rebuild a fresh snapshot to reclaim space when a caller
wants to (e.g. after bulk deletions), but it's never on a hot path.

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

- **Emit**: walk `cards`; `Pristine` → `out.push_str(&source[span])`; `Owned` →
  render tokens. Byte-identical when unedited; O(bytes) always.
- **add_cell(text)**: validate it's one card of the right kind, `lex_card` it
  (µs), find the insert position (end of the matching block), `cards.insert`,
  assign a fresh slot, update the id index. No reparse.
- **remove_cell(id)**: index → slot → position, `cards.remove`, drop from index.
  (Also removes the card's trailing blank/continuation lines as their own cards
  if they belong to it — layout preserved.)
- **set value / renumber a token**: `set_token_text(card, token, ...)` promotes
  the card to `Owned` (if pristine) and rewrites the one token. Every other card
  keeps its verbatim emit.
- **renumber_* (whole model)**: iterate `cards` once, scan each cell/surface's
  tokens for definitions and references, apply local overrides. Only touched
  cards leave the fast path. Parallelizable (see below).
- **read (typed projection)**: build a `CellView`/`SurfaceView` on demand from
  the card's tokens — same projection logic as today, now over a card-local
  token slice. Reads are `&self`; writes are addressed by slot through `Model`,
  so two cells can be read at once without borrow conflicts.
- **validate / extract_universe / merge**: whole-model passes over `cards`,
  building new `Cst`s where needed. `merge`/`extract` construct a new model from
  selected cards — a `Vec<Card>` splice, not string surgery.

## Parallelism (how we reach the stretch targets)
Cards are independent, which makes the expensive whole-model passes data-parallel
with `rayon`:
- **Parse**: split source at block/blank boundaries, lex chunks in parallel, then
  stitch card lists. (Lexing is the bulk of parse time.)
- **Renumber**: the per-card scan is `par_iter`; overrides are collected per card
  then applied. The `id → slot` map is read-only during the scan.
- **Emit**: render owned cards in parallel into per-card buffers, then
  concatenate; or chunk the card list and emit chunks in parallel.

Single-threaded must already meet the base budget; parallelism buys the stretch
goals and headroom for even bigger inputs.

## Crate layout (unchanged — it's good)
- **`migjorn-syntax`** — lexer + the per-card CST (`Cst`, `Card`, `Token`,
  `SyntaxKind`). Owns losslessness and structural editing primitives.
- **`migjorn`** — the typed `Model`: projection, editing, renumber, compose,
  validate. The single public facade.
- **`migjorn-py`** — PyO3 + maturin `abi3` bindings mirroring `api/migjorn.pyi`.

## What this design *deletes*
Complexity that existed only to work around the flat arena's inability to edit
structurally:
- `OwnedCell` + `promote_cell` + the monotonic-slot promotion bookkeeping.
- Emit-only splices + `has_pending_splices` + lazy `materialize()` + the
  `debug_assert` invariant that "the CST is the sole source of truth after
  materialize."
- The `Model` / `ModelView` read-write split that existed to force a
  `materialize()` before any read.

With cards as the unit, the card list is always the current, consistent state.
There is nothing to materialize.

## Risks & mitigations
- **Per-card memmove on insert/remove is O(num_cards).** For ~1M cards a memmove
  of `Card` structs (a few dozen bytes each) is single-digit milliseconds — fine
  for interactive edits, and far below the current 2.4 s. If a workload does
  *millions* of structural edits, offer a batch API (`add_cells(iter)`) that does
  one memmove. (Do not prematurely switch to a gap buffer / linked structure;
  measure first.)
- **Immutable snapshot retains removed bytes.** Bounded by `compact()`; only
  matters under heavy deletion, and is opt-in.
- **`Owned` cards lose the shared-arena locality.** Only edited cards pay it, and
  they are by definition a tiny fraction; the untouched majority stay pristine.
