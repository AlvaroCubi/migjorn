# /// script
# requires-python = ">=3.9"
# dependencies = ["crunchy-mcnp"]
#
# [tool.uv.sources]
# crunchy-mcnp = { path = "..", editable = false }
# ///
"""A guided tour of crunchy, runnable with a single command:

    uv run crates/crunchy-py/examples/crunchy_demo.py

uv reads the inline metadata above, builds the local `crunchy-mcnp` package
(compiling the Rust extension via maturin) into an isolated environment, and
runs this script -- no manual `pip install` / `maturin develop` needed.

For the interactive notebook version, see `crunchy_demo.ipynb` and the uv
commands in this crate's README.
"""

import crunchy

MODEL = """\
Crunchy demo: a small pin-cell-ish model
c --- cells ---
1 1 -10.5 -1 imp:n=1                  $ fuel pin
2 2 -1.0  1 -2 imp:n=1                $ water gap
3 0 (2 -3) #4 imp:n=1                 $ moderator minus the insert
4 3 -2.7 -40 imp:n=1 imp:p=1 &
     vol=8.0                          $ aluminium insert (macrobody)
5 0 3 : 20 imp:n=0                    $ graveyard
10 like 1 but mat=4 rho=-11.3         $ second pin, different fuel

c --- surfaces ---
1 CZ 0.5
2 CZ 0.6
3 CZ 5.0
20 SO 100
30 1 PZ 50                            $ surface positioned by transform 1
40 RPP -1 1 -1 1 -1 2

c --- data ---
m1 92235.80c 0.04 92238.80c 0.96      $ enriched uranium
m2 1001.31c 2 8016.31c 1              $ light water
mt2 lwtr.10t
m3 13027.80c 1                        $ aluminium
m4 92235.80c 0.20 92238.80c 0.80
tr1 0 0 5
mode n
sdef pos=0 0 0 erg=2.0
f4:n 1 2 4
"""


def rule(title):
    print(f"\n=== {title} ===")


def cell_line(text, cell_id):
    for line in text.splitlines():
        toks = line.split()
        if toks and toks[0] == str(cell_id):
            return line
    return None


def main():
    print("crunchy version:", crunchy.__version__)
    model = crunchy.parse(MODEL)

    rule("parse")
    print(repr(model))
    print("diagnostics:", model.diagnostics)

    rule("surfaces")
    for s in model.surfaces:
        tr = f"  (TR{s.transform})" if s.transform else ""
        print(f"{s.id:>3}  {s.kind:<4} {s.coeffs}{tr}")

    rule("cells")
    for c in model.cells:
        if c.like is not None:
            print(f"{c.id:>3}  LIKE {c.like} BUT ...")
            continue
        kind = "void" if c.is_void else f"mat {c.material} @ {c.density}"
        extra = f"  complements #{c.cell_refs}" if c.cell_refs else ""
        print(f"{c.id:>3}  {kind:<14} surfaces {c.signed_surfaces}{extra}")

    rule("materials & transforms")
    for m in model.materials:
        comp = "  ".join(f"{z}={frac}" for z, frac in m.entries)
        print(f"m{m.id:<2} {comp}")
    for t in model.transforms:
        print(f"tr{t.id}  displacement {t.displacement}")

    rule("lossless round-trip")
    assert str(model) == MODEL
    print("byte-for-byte lossless:", str(model) == MODEL)

    rule("whole-geometry renumbering")
    print("cell 3 before:", cell_line(str(model), 3))
    model.offset_surfaces(1000)  # every surface +1000 (defs + refs)
    model.renumber_cells(lambda n: n + 900)  # every cell   +900  (defs + #n + LIKE)
    print("cell 3 after: ", cell_line(str(model), 903))

    edited = str(model)
    print("comment preserved:      ", "$ moderator minus the insert" in edited)
    print("continuation preserved: ", "imp:p=1 &" in edited)
    print("surface def renumbered:  ", "1040 RPP -1 1 -1 1 -1 2" in edited)
    print("LIKE base renumbered:    ", "910 like 901 but" in edited)

    rule("per-card editing")
    # Live handles: explore by card text, then edit values in place.
    demo = crunchy.parse(MODEL)
    for cell in demo.cells:
        if "$ fuel pin" in cell.text:
            cell.material = 5
            cell.density = -10.9
    print(cell_line(str(demo), 1))

    rule("structural geometry editing")
    c = demo.cell(2)
    print("before:", c.signed_surfaces)
    c.add_surface(-3)  # intersect the region with -3
    c.add_complement(4)  # AND in  #4
    c.remove_surface(1)  # drop surface 1 (either sense)
    print("after: ", c.signed_surfaces, " complements", c.cell_refs)
    print(cell_line(str(demo), 2))

    rule("building & removing cards")
    demo2 = crunchy.parse(MODEL)
    demo2.add_surface("50 SO 200")
    demo2.add_material("m9 6000 1")
    new = demo2.add_cell("20 9 -2.0 -50")  # live handle to the new cell
    new.add_surface(1)  # ...editable like any other cell
    print(cell_line(str(demo2), 20))
    print("materials:", demo2.num_materials, " surfaces:", demo2.num_surfaces)
    print("validate:", demo2.validate())  # [] -- consistent
    demo2.remove_surface(50)  # break a reference on purpose
    print("after removing surface 50:", demo2.validate())


if __name__ == "__main__":
    main()
