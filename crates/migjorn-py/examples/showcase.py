"""A self-contained tour of the migjorn Python API.

Runs out of the box (no external model file needed):

    python crates/migjorn-py/examples/showcase.py

It parses a small model, reads it as typed cells/surfaces/materials/transforms,
edits it (value + structural + renumber), composes with another model, and shows
that emission is lossless except where changed. For the large-model / interactive
workload the design targets, point the Rust bench at a big file instead
(`cargo run --release -p migjorn --example bench -- big.mcnp`).
"""

from __future__ import annotations

import migjorn

MODEL = """\
Example reactor pin
1 1 -10.2 -1 u=1 imp:n=1   $ fuel
2 0 1 -2 u=1 imp:n=1       $ gap
3 2 -6.5 2 u=1 imp:n=1     $ clad
10 0 -3 fill=1 imp:n=1     $ pin cell in the lattice
99 0 3 imp:n=0             $ outside world

1 CZ 0.41
2 CZ 0.42
3 CZ 0.60

m1 92235.71c 0.04 92238.71c 0.96   $ fuel
m2 40090.71c 1                      $ zirconium clad
tr1 0 0 5
"""


def rule(title: str) -> None:
    print(f"\n--- {title} " + "-" * (60 - len(title)))


def main() -> None:
    model = migjorn.parse(MODEL)

    rule("parse & explore (O(1) id lookup, typed projection)")
    print(
        f"{model.num_cells} cells, {model.num_surfaces} surfaces, "
        f"{model.num_materials} materials, {model.num_transforms} transforms"
    )
    fuel = model.cell(1)
    print(
        f"cell 1: material={fuel.material} density={fuel.density} "
        f"void={fuel.is_void} surfaces={fuel.signed_surfaces} u={fuel.universe}"
    )
    print(f"cell 1 imp:n = {fuel.param('imp:n').value!r}")
    print(f"surface 3: kind={model.surface(3).kind} coeffs={model.surface(3).coeffs}")
    print(f"material 1 entries: {model.material(1).entries}")
    print(f"transform 1 displacement: {model.transform(1).displacement}")
    print(f"universes declared: {model.universe_ids()}")

    rule("in-card value edits (splice one card; no reparse)")
    model.cell(2).material = 5  # void -> real: a density field appears
    model.cell(2).density = -0.001  # helium gap
    print(
        f"cell 2 now: material={model.cell(2).material} "
        f"density={model.cell(2).density} void={model.cell(2).is_void}"
    )
    model.cell(3).set_param("imp:n", "2")
    model.cell(1).add_param("vol=1.5")
    print(f"cell 1 after add_param: {model.cell(1).text.strip()!r}")

    rule("structural edits (local Vec ops; other handles stay valid)")
    clad = model.cell(3)  # a live handle
    new = model.add_cell("4 2 -6.5 -1 3 u=1 imp:n=1  $ extra clad ring")
    print(f"added cell {new.id}; num_cells now {model.num_cells}")
    print(f"handle to cell 3 still valid after the add: id={clad.id}")
    model.remove_cell(99)
    print(f"removed cell 99 -> lookup returns {model.cell(99)}")

    rule("renumbering (definitions AND references move together)")
    model.offset_surfaces(1000)  # constant shift
    print(
        f"surface def + cell ref: {model.surface(1001).text.strip()!r} | "
        f"cell 1 surfaces={model.cell(1).signed_surfaces}"
    )
    model.renumber_cells({1: 101})  # dict form (unmapped unchanged)
    print(f"cell 1 renumbered to {model.cell(101).id}")

    rule("validate & compose")
    problems = model.validate()
    print(
        f"validate: {len(problems)} problem(s)"
        + (f" (first: {problems[0]})" if problems else "")
    )

    other = migjorn.parse("Bolt-on\n500 0 -900 imp:n=1\n\n900 SO 50\n\nm50 1001 1\n")
    model.merge([other])
    print(
        f"after merge: {model.num_cells} cells, cell 500 present: "
        f"{model.cell(500) is not None}"
    )

    pin = migjorn.parse(MODEL).extract_universe(1)
    print(f"extract_universe(1): {pin.num_cells} cells, {pin.num_surfaces} surfaces")

    rule("lossless emission")
    clean = migjorn.parse(MODEL)
    assert clean.to_source() == MODEL, "round-trip must be byte-identical"
    print("parse(t).to_source() == t  [OK]  (byte-for-byte)")
    print("first 2 lines of the edited model:")
    for line in model.to_source().splitlines()[:2]:
        print(f"  {line}")


if __name__ == "__main__":
    main()
