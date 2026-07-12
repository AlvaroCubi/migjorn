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


def test_card_text_includes_inline_comment():
    model = crunchy.parse(MODEL)
    c = model.cell(1)
    assert "$ fuel" in c.text  # inline comment is part of the card text
    assert "1 1 -1.0" in c.text
    # A surface card exposes its text too.
    assert model.surface(1).text.strip().startswith("1 SO 5")


def test_edit_material_and_density_in_place():
    model = crunchy.parse(MODEL)
    # The stated objective: explore per card, then edit material + density.
    for cell in model.cells:
        if "$ fuel" in cell.text:
            cell.material = 124
            cell.density = 7.93
    out = str(model)
    assert "1 124 7.93 -1 2 #3 imp:n=1 $ fuel" in out
    # The edit is visible through a fresh lookup (handles are live).
    assert model.cell(1).material == 124
    assert model.cell(1).density == 7.93
    # Everything else is byte-identical.
    assert "2 0 1 imp:n=0" in out
    assert "3 0 -2 imp:n=1" in out
    assert "m1 1001.31c 0.667 8016.31c 0.333" in out


def test_edit_requiring_structural_change_raises():
    model = crunchy.parse(MODEL)
    # Cell 2 is void: it has no density field to set.
    try:
        model.cell(2).density = 1.0
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError setting density on a void cell")
    # Void -> real material would add a density field.
    try:
        model.cell(2).material = 5
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError on a void/non-void change")


def test_num_materials_and_transforms():
    model = crunchy.parse(MODEL)
    assert model.num_materials == 1
    assert model.num_transforms == 1


def test_add_and_remove_surface_in_geometry():
    model = crunchy.parse(MODEL)
    c = model.cell(1)  # "1 1 -1.0 -1 2 #3 imp:n=1 $ fuel"
    c.add_surface(-7)  # intersect with surface 7 (negative sense)
    assert -7 in c.signed_surfaces
    out = str(model)
    assert "-7" in out
    # The parameter tail (incl. inline comment) survives the restructure.
    assert "imp:n=1" in out
    # Other cards untouched, and the model still re-parses cleanly.
    assert "2 0 1 imp:n=0" in out
    assert crunchy.parse(out).diagnostics == []

    # Remove it again.
    assert c.remove_surface(7) is True
    assert -7 not in c.signed_surfaces


def test_add_and_remove_complement():
    model = crunchy.parse(MODEL)
    c = model.cell(2)  # "2 0 1 imp:n=0" (void)
    c.add_complement(1)
    assert 1 in c.cell_refs
    assert "#1" in str(model)
    assert c.remove_complement(1) is True
    assert 1 not in c.cell_refs


def test_geometry_edit_refused_when_it_would_empty():
    model = crunchy.parse(MODEL)
    c = model.cell(3)  # "3 0 -2 imp:n=1" — single surface
    try:
        c.remove_surface(2)
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError removing the last surface")


def test_add_cell_returns_editable_handle():
    model = crunchy.parse(MODEL)
    c = model.add_cell("10 0 -1")  # new void cell in the cell block
    assert c.id == 10 and c.is_void
    c.add_surface(2)  # edit the freshly-added cell like any other
    assert 2 in c.surface_ids
    out = str(model)
    assert "10 0 -1 2" in out
    # Existing cards untouched; model still re-parses cleanly.
    assert "1 1 -1.0 -1 2 #3 imp:n=1 $ fuel" in out
    assert model.num_cells == 4
    assert crunchy.parse(out).diagnostics == []


def test_add_surface_and_material():
    model = crunchy.parse(MODEL)
    s = model.add_surface("9 SO 12.0")
    assert s.id == 9 and s.kind == "SO" and s.coeffs == [12.0]
    m = model.add_material("m2 26000 -1.0")
    assert m.id == 2
    assert model.num_surfaces == 3
    assert crunchy.parse(str(model)).diagnostics == []


def test_remove_cell_and_validate():
    model = crunchy.parse(MODEL)
    # Cell 1 has a #3 complement; removing cell 3 makes it dangle.
    assert model.validate() == []
    assert model.remove_cell(3) is True
    assert model.num_cells == 2
    problems = model.validate()
    assert any("missing cell 3" in p for p in problems), problems
    # Removing a missing cell is a no-op.
    assert model.remove_cell(999) is False


def test_add_rejects_bad_text():
    model = crunchy.parse(MODEL)
    try:
        model.add_cell("this is not a cell")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for malformed cell text")


def test_renumber_materials_and_transforms():
    model = crunchy.parse(MODEL)
    model.renumber_materials({1: 101})
    out = str(model)
    assert "m101 1001.31c 0.667" in out  # Mn definition
    assert "1 101 -1.0" in out  # cell 1's material field
    # Transforms: the model has tr1 and surfaces have no transform field here,
    # so just check the definition renumbers.
    model.renumber_transforms(lambda n: n + 5)
    assert "tr6 0 0 5" in str(model)


if __name__ == "__main__":
    test_parse_and_lossless()
    test_typed_access()
    test_renumber_offset_lossless_elsewhere()
    test_renumber_with_dict_and_callable()
    test_renumber_callable_error_propagates()
    test_card_text_includes_inline_comment()
    test_edit_material_and_density_in_place()
    test_edit_requiring_structural_change_raises()
    test_num_materials_and_transforms()
    test_add_and_remove_surface_in_geometry()
    test_add_and_remove_complement()
    test_geometry_edit_refused_when_it_would_empty()
    test_add_cell_returns_editable_handle()
    test_add_surface_and_material()
    test_remove_cell_and_validate()
    test_add_rejects_bad_text()
    test_renumber_materials_and_transforms()
    print("all crunchy binding smoke tests passed")
