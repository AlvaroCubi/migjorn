"""A worked iterative-editing session — the workload the redesign optimizes for.

This is the story the architecture must make *fast and simple*: open a large
model, poke around, make many small structural + value edits interleaved with
reads, and write it back — all in interactive time, on a 380 MB file.

Every operation below is intended to be sub-100 ms on ``big.mcnp`` (most are
microseconds); the whole script well under a second excluding parse. Contrast
with today's engine, where each ``add_cell`` / ``remove_cell`` and each read
after a splice costs ~2.4 s and ~1.6 s respectively (a full-file reparse).
"""

import migjorn

# Parse once (~1 s on 380 MB; the only unavoidable O(file) step).
model = migjorn.Model.from_file("big.mcnp")
print(f"{model.num_cells} cells, {model.num_surfaces} surfaces")

# --- explore: O(1) id lookup, on-demand typed projection --------------------
fuel = model.cell(1000)
print(fuel.material, fuel.density, fuel.signed_surfaces)
print(fuel.param("imp:n").value if fuel.param("imp:n") else "no imp")

# --- iterate and edit in one pass -------------------------------------------
# Bump the importance of every void cell. Reading a param and setting one on
# the same handle must stay consistent with no explicit "flush".
for cell in model.cells():
    if cell.is_void and cell.param("imp:n") is None:
        cell.add_param("imp:n=1")

# --- structural edits: add / remove are local, ~O(num_cards), not a reparse -
new = model.add_cell("9000001 6 -7.85 -5 6 imp:n=1")   # returns a live handle
new.append_comment("$ added shim")
model.add_surface("9000001 SO 12.0")

model.remove_cell(2000)          # cheap; other handles remain valid
assert model.cell(2000) is None

# A handle to a removed card is the only staleness; using it raises.
# (new is still fine — we removed a *different* card.)
print(new.text)

# --- whole-model renumbering: defs + every reference, consistently ----------
# Shift an imported sub-block out of the way before a merge, say.
model.renumber_surfaces(lambda i: i + 1_000_000 if i < 500 else i)
model.renumber_cells({7: 7007, 8: 8008})   # dict form; unmapped ids unchanged

# --- validate and write back ------------------------------------------------
problems = model.validate()
if problems:
    print(f"{len(problems)} consistency problems, first: {problems[0]}")

# Emission copies untouched cards verbatim and renders only the handful we
# edited — O(bytes), no reparse.
model.save("big_edited.mcnp")
