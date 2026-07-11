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

DECK = """\
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
    deck = crunchy.parse(DECK)

    rule("parse")
    print(repr(deck))
    print("diagnostics:", deck.diagnostics)

    rule("surfaces")
    for s in deck.surfaces:
        tr = f"  (TR{s.transform})" if s.transform else ""
        print(f"{s.id:>3}  {s.kind:<4} {s.coeffs}{tr}")

    rule("cells")
    for c in deck.cells:
        if c.like is not None:
            print(f"{c.id:>3}  LIKE {c.like} BUT ...")
            continue
        kind = "void" if c.is_void else f"mat {c.material} @ {c.density}"
        extra = f"  complements #{c.cell_refs}" if c.cell_refs else ""
        print(f"{c.id:>3}  {kind:<14} surfaces {c.signed_surfaces}{extra}")

    rule("materials & transforms")
    for m in deck.materials:
        comp = "  ".join(f"{z}={frac}" for z, frac in m.entries)
        print(f"m{m.id:<2} {comp}")
    for t in deck.transforms:
        print(f"tr{t.id}  displacement {t.displacement}")

    rule("lossless round-trip")
    assert str(deck) == DECK
    print("byte-for-byte lossless:", str(deck) == DECK)

    rule("whole-geometry renumbering")
    print("cell 3 before:", cell_line(str(deck), 3))
    deck.offset_surfaces(1000)             # every surface +1000 (defs + refs)
    deck.renumber_cells(lambda n: n + 900)  # every cell   +900  (defs + #n + LIKE)
    print("cell 3 after: ", cell_line(str(deck), 903))

    edited = str(deck)
    print("comment preserved:      ", "$ moderator minus the insert" in edited)
    print("continuation preserved: ", "imp:p=1 &" in edited)
    print("surface def renumbered:  ", "1040 RPP -1 1 -1 1 -1 2" in edited)
    print("LIKE base renumbered:    ", "910 like 901 but" in edited)


if __name__ == "__main__":
    main()
