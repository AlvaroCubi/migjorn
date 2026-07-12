"""Functional tests for the crunchy Python bindings.

Run against an installed build (e.g. `maturin develop` then `pytest`), or
directly with `python test_bindings.py` for a quick smoke check.
"""

import crunchy

MODEL = """\
Example model
1 1 -1.0 -1 2 #3 imp:n=1 $ fuel
2 0 1 imp:n=0
3 0 -2 imp:n=1

1 SO 5
2 PX 0

m1 1001.31c 0.667 8016.31c 0.333
tr1 0 0 5
sdef pos=0 0 0
"""


def test_parse_and_lossless():
    model = crunchy.parse(MODEL)
    assert str(model) == MODEL  # lossless round-trip
    assert model.diagnostics == []
    assert model.num_cells == 3
    assert model.num_surfaces == 2


def test_typed_access():
    model = crunchy.parse(MODEL)

    s = model.surface(1)
    assert s.id == 1 and s.kind == "SO" and s.coeffs == [5.0]

    c = model.cell(1)
    assert c.material == 1 and c.density == -1.0
    assert c.is_void is False
    assert sorted(c.surface_ids) == [1, 2]
    assert c.cell_refs == [3]  # from #3

    m = model.material(1)
    assert m.id == 1
    assert m.entries[0] == ("1001.31c", 0.667)

    tr = model.transform(1)
    assert tr.id == 1 and tr.displacement == (0.0, 0.0, 5.0)

    names = {d.name for d in model.data_cards}
    assert {"M1", "TR1", "SDEF"} <= names


def test_renumber_offset_lossless_elsewhere():
    model = crunchy.parse(MODEL)
    model.offset_surfaces(100)
    out = str(model)
    # Definitions shifted.
    assert "101 SO 5" in out
    assert "102 PX 0" in out
    # References shifted, sense preserved, spacing/comments intact.
    assert "-101 102 #3 imp:n=1 $ fuel" in out
    assert out.split("\n")[0] == "Example model"  # title unchanged


def test_renumber_with_dict_and_callable():
    model = crunchy.parse(MODEL)
    model.renumber_cells({1: 10, 3: 30})
    out = str(model)
    assert "10 1 -1.0" in out
    assert "30 0 -2" in out
    assert "#30" in out  # the #3 complement followed cell 3 -> 30

    model2 = crunchy.parse(MODEL)
    model2.renumber_surfaces(lambda n: n * 1000)
    assert "1000 SO 5" in str(model2)


def test_renumber_callable_error_propagates():
    model = crunchy.parse(MODEL)
    try:
        model.renumber_surfaces(lambda n: "not an int")
    except Exception:
        pass
    else:
        raise AssertionError("expected an error from a bad mapping")


if __name__ == "__main__":
    test_parse_and_lossless()
    test_typed_access()
    test_renumber_offset_lossless_elsewhere()
    test_renumber_with_dict_and_callable()
    test_renumber_callable_error_propagates()
    print("all crunchy binding smoke tests passed")
