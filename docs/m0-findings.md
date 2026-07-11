# M0 — Performance spike findings

**Question:** Can we build a *lossless* concrete syntax tree over MCNP input fast
enough to hit the target (1M surfaces < 1s; 10M-line files)?

**Answer:** Yes — with a custom flat-arena tree. `rowan` is too slow for the hot
path and is dropped in favour of the custom design the plan named as the fallback.

## Method

Measured on the real deck `resources/untracked/big.mcnp` (360 MB, 6.47M lines)
and its isolated surface block (1,204,952 lines ≈ 1.2M surfaces), release build,
`opt-level=3 / lto=thin / codegen-units=1`.

Two tree builders over the same lexer:
- **rowan** `GreenNodeBuilder` (interns every token via a hashmap).
- **flat** parallel `Vec`s of `(kind, len)` with token text recovered as spans
  into the original source (no copy, no interning).

## Results

| Workload | Stage | rowan | flat |
|---|---|---|---|
| Surface block (1.2M surfaces, 66 MB, 8.9M tokens) | lex | 57 ms (1111 MB/s) | 57 ms |
| | tree build | 915 ms (69 MB/s) | **76 ms (839 MB/s)** |
| | lex + build | 972 ms | **133 ms** |
| | round-trip emit | 687 ms | 48 ms |
| Full file (6.47M lines, 360 MB, 91.6M tokens) | lex | 428 ms (842 MB/s) | 428 ms |
| | tree build | 4.88 s (74 MB/s) | **1.08 s (332 MB/s)** |
| | lex + build | 5.31 s | **1.51 s** |

- Read (fs → String, UTF-8 validated): ~2.9 GB/s (120 ms for 360 MB).
- Lexer throughput ~0.85–1.1 GB/s — this is the true floor of parse cost.
- Both trees round-trip **byte-for-byte** (`to_string() == src`).

## Decision

- **Layer 1 = custom flat-arena lossless tree.** rowan's per-token hashmap
  interning is the entire bottleneck (~10× the lexer). The flat design meets the
  1M-surface target with ~7× headroom (133 ms) and cuts full-file build 3.5×.
- Token text is **not** stored/interned — tokens are `(kind, len)`; text is a
  span into the source (implicit start = prefix sum of lengths).
- Keep the lexer as-is; it is not the bottleneck.

## Follow-ups for M1+

- Generalise the prototype (currently line-grouped) to a real nested node arena
  (blocks → cards → sub-nodes) with children stored as index ranges.
- Editing (M5): a flat arena makes structural splices harder than rowan's
  persistent trees. Keep the fast parse/read path allocation-lean; support edits
  by rebuilding affected spans or a hybrid overlay — edits are rare vs reads.
- Further build-speed headroom if needed: pack arrays, drop `line_starts` on the
  read-only path, or parse straight to the typed model without materialising all
  tokens.
