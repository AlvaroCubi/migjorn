# /// script
# requires-python = ">=3.9"
# dependencies = ["migjorn"]
#
# [tool.uv.sources]
# migjorn = { path = "..", editable = false }
# ///
"""Split a model into universe sub-models, edit them, and recompose.

    uv run crates/migjorn-py/examples/universe_compose.py

Demonstrates the model-assembly API: reading a cell's ``u=``/``fill=``, carving
a universe (and the level-0 shell) into standalone models, and merging disjoint
models back together with id-collision detection.
"""

import migjorn

MODEL = """\
Lattice model: a filled shell over one universe
1 0 -1 fill=10                        $ shell, filled by universe 10
2 1 -1.0 -2 3 u=10                    $ universe-10 fuel
3 0 2 -3 u=10                         $ universe-10 clad gap

1 SO 5
2 PX -1
3 PX 1

m1 1001 1
sdef pos=0 0 0
"""


def main() -> None:
    model = migjorn.parse(MODEL)
    print("universes in the model:", model.universe_ids())

    shell_cell = model.cell(1)
    print("cell 1 fill universe:", shell_cell.fill.universe)
    print("cell 2 universe:", model.cell(2).universe)

    # Carve universe 10 into its own runnable model: its cells, the surfaces
    # they reference, and only the data cards they need (m1 and sdef here).
    u10 = model.extract_universe(10)
    print("\nextracted universe 10 ->", u10)
    print(u10.to_source())

    # Edit the extracted universe in place -- edits are lossless and visible on
    # re-emission.
    u10.cell(2).append_comment("edited in the sub-model")

    # Compose a new model: start from the level-0 shell (which owns the global
    # data such as the source) and merge in a fresh, disjoint filler universe.
    # Disjoint ids across the merged models are required; the shell keeps the
    # authoritative data block.
    shell = model.extract_level0()
    filler20 = migjorn.parse("universe 20\n20 0 -20 u=20\n\n20 SO 2\n")
    shell.merge([filler20])
    print("recomposed ->", shell)
    assert shell.validate() == [], shell.validate()

    # A merge that would duplicate an id is refused and leaves the model as-is.
    clash = migjorn.parse("dup\n1 0 -9\n\n9 SO 1\n")
    try:
        shell.merge([clash])
    except migjorn.MergeError as e:
        print("merge refused:", e)


if __name__ == "__main__":
    main()
