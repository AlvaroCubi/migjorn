# migjorn: API improvements worth considering

Notes from building gitronics against **migjorn 0.3.8**. These are not bugs — the crate
does what it says. They are places where gitronics has to reach around the API, and each
one is backed by code that exists in this repo today.

Ordered by how much they would actually change gitronics.

## What already works well, for context

Worth stating, because the list below is one-sided by construction:

- **Losslessness is real and load-bearing.** `to_source()` being a plain concatenation of
  `card.text()` is what let gitronics stream a 376 MB model to disk without allocating it.
- **Slot-based editing survives edits.** Snapshotting slots then mutating by slot is the
  pattern in both `add_fill_cards_to_envelopes` and `replace_fills_with_placeholders`, and
  it holds up.
- **`Diagnostic` is the right shape** — typed severity, message, span. Item 3 below is
  really "apply this existing good idea in two more places".
- **The view types** (`CellView::universe`, `::material`, `::surface_ids`) meant gitronics
  never had to parse a cell card itself.

---

## 1. `Fill` can be read but not written

**The asymmetry:** `CellView::fill()` returns a typed `Fill { universe, starred,
transform }`, but there is no way to write one back. `Fill` has no constructor, no
`Display`, and there is no `set_fill`. Anything that puts a fill onto a cell must build
the MCNP text by hand and go through `add_cell_param`.

**What that costs gitronics** (`src/build_model.rs`):

```rust
let fill_card_text = if let Some(transform_without_star) = transform.strip_prefix('*') {
    format!("*fill={universe_id} {transform_without_star}")
} else {
    format!("fill={universe_id} {transform}")
};
envelope_structure.add_cell_param(slot, fill_card_text.trim())
```

gitronics is re-deriving `starred` by string-prefix inspection, re-assembling the
parenthesised transform, and hoping the spacing is right — for a value migjorn already
models precisely.

**This caused a real bug.** `Fill::transform` holds the parentheses verbatim (`(30)`, not
`30`), which the doc comment does say. gitronics' `migrate` wrapped them a second time and
emitted `fill=2 ((30))` into every migrated model with a transformed fill. Nothing in the
type system objected, because the round trip goes through `String`. A `Display for Fill`
would have made the correct code the obvious code, and `set_fill` would have made the
mistake unrepresentable.

**Sketch:**

```rust
impl Fill {
    pub fn new(universe: i64) -> Fill;                    // starred: false, transform: None
    pub fn with_transform(self, parenthesised: &str) -> Fill;
    pub fn starred(self, yes: bool) -> Fill;
}
impl fmt::Display for Fill { /* "*fill=2 (30)" or "fill=2" */ }

impl Model {
    pub fn set_fill(&mut self, slot: u32, fill: &Fill) -> Result<(), EditError>;
    pub fn remove_fill(&mut self, slot: u32) -> Result<bool, EditError>;
}
```

`remove_fill` would also give `migrate` a purpose-built call instead of
`remove_cell_param(slot, "fill")`, which is where the stranded-`*` bug lived.

One design question worth deciding deliberately: whether `transform` stays "parenthesised
verbatim" or becomes a small enum (`Named(i64)` / `Inline(String)`). The enum is harder to
misuse; the string is harder to lose information through. Either is better than today's
`Option<String>` that callers must both parse and re-emit.

**Priority: highest.** It is the only item here that has already produced corrupt output.

---

## 2. No first-class way to build a model of just one block

**What gitronics does** (`src/build_model.rs`), to get the configured data cards into a
model so `merge` can absorb and index them:

```rust
to_merge.push(Model::parse(&format!(
    "gitronics data cards\n\n\n{data_text}\n"
)));
```

Three newlines: one to end the title, then an empty cell block and an empty surface block,
so that positional segmentation lands `data_text` in the data block. It works, and there
is a test pinning it — but it is an incantation that depends on segmentation internals,
and the failure mode is silent and severe:

| source | resulting card kinds |
| --- | --- |
| `"t\n\n\n" + data` | `Title, Blank, Blank, Data, Data` — correct |
| `"t\n\n" + data` | `Title, Blank, **Surface**, **Surface**` |
| `"t\n" + data` | `Title, **Cell**, **Cell**` |

With one newline wrong, `M1` and `SDEF` become surface cards. `validate()` returns **no
problems** for that model, and `num_materials()` is `0`. The only signal is
`diagnostics()` (two `Error`-severity "surface card has no readable id"), which a caller
constructing a model inline has no particular reason to check.

**Sketch:**

```rust
impl Model {
    /// A model whose only content cards are data cards.
    pub fn from_data_cards(text: &str) -> Model;
}
```

or, more generally, a builder that makes block membership explicit rather than positional:

```rust
Model::builder()
    .title("gitronics data cards")
    .data(data_text)
    .build()
```

Either removes the incantation and the class of silent misfiling with it. The builder also
gives `extract_universe` / `extract_level0` a natural counterpart for composing models up
rather than only slicing them down.

**Priority: high.** Low effort, and it closes a corruption path rather than an annoyance.

---

## 3. `validate()` and `merge()` return `Vec<String>`

The crate already has the right pattern in `Diagnostic`. These two do not use it:

```rust
pub fn validate(&self) -> Vec<String>;
pub fn merge(&mut self, others: Vec<Model>) -> Result<(), Vec<String>>;
```

The strings are structured underneath — `"cell {id}: references undefined surface {s}"`,
`"duplicate {label} id {id}"` — but the structure is thrown away at the boundary and the
caller cannot get it back without parsing English.

**What gitronics does with them:** joins them with `"\n"` and puts the blob in an error
message. That is all it *can* do. Concretely, it means gitronics cannot:

- report "3 duplicate cell ids, 1 duplicate material id" instead of a wall of lines;
- name the two components that collided (the id is there; the owning models are not);
- put collisions into `build_report.json` as data, which is exactly the sort of thing the
  manifest exists for;
- distinguish "undefined surface" from "undefined universe" to give a targeted hint.

**Sketch:**

```rust
pub enum Problem {
    UndefinedSurface { cell: i64, surface: i64 },
    UndefinedMaterial { cell: i64, material: i64 },
    UndefinedCell { cell: i64, referenced: i64 },
    UndefinedUniverse { cell: i64, universe: i64 },
    UndefinedTransform { surface: i64, transform: i64 },
}
pub fn validate(&self) -> Vec<Problem>;

pub struct Collision { pub kind: IdKind, pub id: i64, /* maybe: models: Vec<usize> */ }
pub fn merge(&mut self, others: Vec<Model>) -> Result<(), Vec<Collision>>;
```

with `Display` on both so existing callers keep a one-line migration. If the index in
`others` can be carried on `Collision`, that is the single most useful field for a
compositional tool — "these two fillers both define cell 100" is the message a user needs,
and gitronics currently cannot produce it.

**Priority: high**, mostly for `merge`. Id collisions are the error gitronics users hit
most, and it is the one where the current message is least actionable.

---

## 4. No streaming output

`to_source()` allocates the entire source. For a 376 MB model that is a full extra copy at
the moment of writing, on top of the CST.

gitronics now hand-rolls the streaming (`src/build_model.rs::write_assembled`), which is
only safe because `to_source` is documented as a plain concatenation:

```rust
for card in model.cst().cards() {
    writer.write_all(card.text().as_bytes())?;
}
```

That is fine, but every caller that cares about memory has to rediscover both the trick
and the guarantee that licenses it.

**Sketch:**

```rust
impl Cst   { pub fn write_source(&self, w: &mut impl io::Write) -> io::Result<()>; }
impl Model { pub fn write_source(&self, w: &mut impl io::Write) -> io::Result<()>; }
```

with `to_source()` kept as the convenience wrapper. Worth considering whether the parallel
emit path above `PARALLEL_EMIT_THRESHOLD` can still be used here (emit chunks in parallel,
write them in order) — if so this is faster *and* smaller than `to_source`.

**Priority: medium.** Not blocking; removes a copy and a piece of folklore.

---

## 5. `clear_data_cards` always deep-copies

It clones every kept card into a fresh `Cst`. gitronics calls it once per filler, on every
build, purely to drop each component's own data block:

```rust
let mut to_merge: Vec<Model> = fillers
    .into_iter()
    .map(|(_, model)| model.clear_data_cards())
    .collect();
```

gitronics already consumes the original with `into_iter` so it frees promptly, but each
call still materialises a full clone of the geometry while the original is alive. On the
big model the fillers are the bulk of a 2.8 GB peak.

**Sketch:** an in-place counterpart that drops the data cards by draining rather than
cloning:

```rust
impl Model { pub fn clear_data_cards_in_place(&mut self); }
```

(Naming aside — `retain_geometry` might read better.) `Cst::remove_slot` already exists, so
this may be mostly a matter of exposing the owned path. Keep the cloning version; the
`&self` signature is genuinely useful when the original is still needed.

**Priority: medium.** Straightforward win for anyone composing many components.

---

## 6. No way to insert cards at a position

gitronics stamps a provenance banner — six comment lines — immediately after the model
title. There is `set_title`, and `append_cell_comment` for a specific cell, but nothing
that adds cards at a chosen point in the document. `Cst::insert_at` exists but `Model`
exposes only `cst(&self)`, so it is unreachable.

So gitronics does the insertion during the write, by finding the first newline in the
concatenated card stream and splicing around it:

```rust
match newline_at {
    Some(i) => {
        writer.write_all(&head.as_bytes()[..=i])?; // title line, including '\n'
        writer.write_all(banner.as_bytes())?;
        ...
```

That is text surgery on a structure that knows exactly where its title card ends.

**Sketch:** either the narrow, obvious one —

```rust
impl Model { pub fn insert_comment_after_title(&mut self, text: &str) -> Result<(), EditError>; }
```

— or the general one, which also covers "put this cell here, not at the end of the block":

```rust
impl Model {
    pub fn insert_card_after(&mut self, slot: u32, kind: CardKind, text: &str) -> Result<u32, EditError>;
}
```

The general form subsumes `add_cell_after` and would have made the starred-`*` bug
workaroundable downstream rather than blocking on a release.

**Priority: medium.** A provenance header is a common thing to want from a tool that
assembles decks.

---

## 7. `Model` does not implement `Debug`

`#[derive(Clone)]` only. Any test holding a `Model` inside a `Result`, `Vec` or tuple loses
`unwrap_err()`, `assert_eq!`, and `#[derive(Debug)]` on wrapper types.

gitronics hits this in `order_fillers_by_cell_id`'s test, which returns
`Result<Vec<(FillerName, Model)>, _>` and so has to write:

```rust
// `Model` is not `Debug`, so match rather than `unwrap_err`.
let Err(err) = order_fillers_by_cell_id(fillers) else {
    panic!("a filler with no cells must be rejected");
};
```

A summary impl is plenty — card count, diagnostic count, title — and deliberately *not* the
whole CST, which nobody wants in a panic message.

**Priority: low, effort trivial.**

---

## Suggested order

1. **#1 `Fill` write path** and **#2 block-scoped construction** — both close silent
   corruption paths, and both are small.
2. **#3 structured `merge` errors** — the biggest improvement to what gitronics can tell a
   user when a build fails.
3. **#7 `Debug`** — five minutes, unblocks ordinary test ergonomics.
4. **#4, #5, #6** — real but not urgent; each removes a workaround gitronics currently
   carries, and #6 is the one that would have let us route around a bug without a release.
