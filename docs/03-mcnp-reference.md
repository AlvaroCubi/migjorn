# 03 — MCNP Input Format: the subset we handle, and the gotchas

This is a practical reference for the format, oriented at what the parser must
get right for losslessness and correct typed projection. It is not the full MCNP
manual. When in doubt, **preserve bytes and record a diagnostic** rather than
guess.

## File structure

```
[MESSAGE block]          (optional; ends at the first blank line)
[blank line]             (only if a MESSAGE block was present)
Title card               (exactly one line — free text)
Cell cards               \
[blank line delimiter]    |
Surface cards             > the three "blocks"
[blank line delimiter]    |
Data cards               /
[optional trailing junk] (anything after the data block)
```

Blocks are separated by a **blank line** (a line that is empty or all
whitespace). This delimiter is load-bearing and must be preserved exactly.

### Gotchas
- The **title** is whatever is on the first card line (after an optional MESSAGE
  block). It can look like anything — including starting with `c` (see
  `title_like_comment`). Do not treat it as a comment.
- A **MESSAGE block** begins with `MESSAGE:` and runs to the first blank line. It
  precedes the title. Rare, but must round-trip.
- Trailing content after the data block (e.g. a second blank line then notes)
  must be preserved verbatim.

## Comments

- **Full-line comment**: a line whose first non-blank character is `c` or `C`
  followed by whitespace or end-of-line. Columns matter loosely (`c` in cols
  1–5); we treat "first non-blank is c/C then space/EOL" as the rule. The whole
  line is a comment.
- **Inline comment**: `$` to end of line — anywhere. Everything from `$` onward
  (including the `$`) is comment trivia attached to the current card.
- Comments are trivia tokens: preserved, never dropped, attached to the card
  they sit on/within.

## Continuations (two independent mechanisms)

A card may span multiple physical lines by either:
1. **Trailing `&`**: a line ending in `&` (before any inline comment) continues
   on the next line.
2. **Blank-column indent**: a line whose first 5 columns are blank continues the
   previous card.

Both appear in real files, sometimes mixed. A comment line (`c ...`) may be
"absorbed" between continuation lines and belongs to the card. Losslessness
means the exact `&`, the exact indentation, and any interleaved comment lines
are all preserved.

## Cell cards

```
<id> <mat> [<density>] <geometry> [params...]      $ inline comment
<id> LIKE <n> BUT <params...>
```
- `id`: positive integer.
- `mat`: material number; `0` means **void** (and then there is **no density**).
- `density`: present only when `mat != 0`. Positive = atom density (atoms/b·cm),
  negative = mass density (g/cm³). It is a float (`-1.0`, `9.81e-1`).
- `geometry`: a boolean region expression (below). Runs until the first
  parameter keyword.
- `params`: `keyword[:particle][=]value` entries — `imp:n=1`, `imp:p=0`, `vol=3`,
  `u=5`, `fill=7 (…)`, `*fill=…`, `trcl=4`, `*trcl=(…)`, `ext:n=0.7`, `pwt=-1`,
  `lat=1`, … A keyword may carry a `*` prefix and a `:particle` designator.
- `LIKE n BUT`: a cell defined as a delta from base cell `n`. No material field,
  no geometry; the only reference is the base cell `n`.

### Geometry expression grammar (MCNP operator precedence)
```
union        := intersection (':' intersection)*        # : is union
intersection := factor (factor)*                        # juxtaposition = AND
factor       := '#' '(' union ')'                       # complement of a region
              | '#' NUMBER                               # complement of a cell
              | '(' union ')'
              | signed-surface                           # e.g. -3, +5, 3
```
- Complement `#` binds tightest, then intersection (space), then union `:`.
- A signed surface: `-n` selects the negative sense, `n` the positive.
- `#n` is a **cell** reference (complement of cell n); `#(...)` complements a
  region whose interior numbers are **surface** references. The distinction
  (a `NUMBER` right after `#` is a cell, else a surface) is essential for
  renumbering — cell renumber must touch `#n` but not surfaces, and vice versa.

## Surface cards

```
[*|+]<id> [<transform>] <mnemonic> <coeffs...>         $ inline comment
```
- Leading `*` = **reflective** boundary; leading `+` = **white** boundary.
  These prefixes are attached to the id and must be preserved (renumbering a
  `+n` white surface keeps the `+`).
- Optional **transform** number before the mnemonic. Negative = **periodic**
  (references a partner surface); sign preserved on renumber.
- `mnemonic`: `PX PY PZ P SO S SX SY SZ CX CY CZ C/X C/Y C/Z KX KY KZ K/X…
  SQ GQ TX TY TZ`, plus **macrobodies** `BOX RPP SPH RCC RHP HEX REC TRC ELL
  WED ARB`.
- `coeffs`: floats. Macrobodies have many; often continued across lines.

## Data cards (typed ones)

- **`Mn`** material: `Mn <zaid> <frac> <zaid> <frac> …`. ZAID may carry a
  library suffix (`1001.31c`). Fraction sign: positive = atomic, negative =
  weight. `Mn` appears in cell material fields; renumber materials touches both.
- **`MTn` / `MXn`**: reference material `n` (thermal scattering / cross-section
  swap). Renumber materials must follow these.
- **`TRn` / `*TRn`** transform: `TRn o1 o2 o3 [rotation…]`. `*TRn` = rotation
  entries are angles in degrees. Referenced by surface transform fields,
  `trcl=`, and single-number `fill= u (n)` groups.
- **`Fn` tallies** and companions (`FCn FMn FSn En Tn Cn SDn DEn DFn …`): the
  trailing digits are the tally number; the last digit gives the type
  (`1,2`→surface bins; `4,6,7,8`→cell bins; `5`→point detector).
  `Model::renumber_tallies` rewrites the tally cards' own ids. It does
  **not** currently rewrite the cell/surface numbers used as tally *bins*
  (e.g. the `1 2 3` in `f4:n 1 2 3`) — `renumber_cells`/`renumber_surfaces`
  only touch cell and surface cards, not `Data` cards. A cell/surface
  renumber on a model with tallies can leave a bin pointing at a
  since-renumbered id; `validate()` does not check for this either. Fixing
  this is tracked as future work, not a current guarantee.

Everything else (`sdef`, `mode`, `nps`, `kcode`, `si/sp/sb`, `print`, `rand`,
`fmesh`, …) is lexed and round-tripped generically and scanned generically by
renumber where relevant, but not given a bespoke typed view unless the API needs
one.

## Data-array shortcuts

Value lists use shorthand that expands positionally:
- `nR` — repeat the previous entry `n` times (`1 3R` → `1 1 1 1`).
- `nI` — interpolate `n` values linearly between neighbours (`1 2I 4` → `1 2 3 4`).
- `nILOG` — logarithmic interpolation.
- `nM` — multiply the previous entry by `n` (`1 2M` → `1 2`).
- `nJ` — jump (`n` defaulted entries).

For **losslessness** we preserve these tokens verbatim — we do **not** expand
them in the source. Typed readers that need expanded values (rare) expand a copy
on demand; the stored card keeps the shorthand.

## Numbers and whitespace

- Integers and floats in the usual forms, including `1e-3`, `1.5E+2`, leading
  `+`/`-`. A number may be immediately followed by a `:` or `(` with no space.
- Whitespace is significant only as a token separator; the *exact* run is
  preserved (we never renormalize spacing on an untouched card).
- Tabs may appear; preserve them.

## Recoverability expectations

Given malformed input the parser must, without panicking:
- unbalanced parens, dangling operators → geometry marked not-well-formed, bytes
  preserved, diagnostic recorded;
- non-numeric where a number is expected → that field not-well-formed, bytes
  preserved;
- too few/many surface coefficients → surface not-well-formed;
- and still round-trip the whole file byte-for-byte.

See `tests/corpus/malformed_recoverable.mcnp`.
