"""Functional tests for the migjorn Python bindings.

Run against an installed build (e.g. `maturin develop` then `pytest`), or
directly with `python test_bindings.py` for a quick smoke check.
"""

import migjorn

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
    model = migjorn.parse(MODEL)
    assert str(model) == MODEL  # lossless round-trip
    assert model.diagnostics == []
    assert model.num_cells == 3
    assert model.num_surfaces == 2


def test_typed_access():
    model = migjorn.parse(MODEL)

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
    model = migjorn.parse(MODEL)
    model.offset_surfaces(100)
    out = str(model)
    # Definitions shifted.
    assert "101 SO 5" in out
    assert "102 PX 0" in out
    # References shifted, sense preserved, spacing/comments intact.
    assert "-101 102 #3 imp:n=1 $ fuel" in out
    assert out.split("\n")[0] == "Example model"  # title unchanged


def test_renumber_with_dict_and_callable():
    model = migjorn.parse(MODEL)
    model.renumber_cells({1: 10, 3: 30})
    out = str(model)
    assert "10 1 -1.0" in out
    assert "30 0 -2" in out
    assert "#30" in out  # the #3 complement followed cell 3 -> 30

    model2 = migjorn.parse(MODEL)
    model2.renumber_surfaces(lambda n: n * 1000)
    assert "1000 SO 5" in str(model2)


def test_renumber_callable_error_propagates():
    model = migjorn.parse(MODEL)
    try:
        model.renumber_surfaces(lambda n: "not an int")
    except Exception:
        pass
    else:
        raise AssertionError("expected an error from a bad mapping")


def test_card_text_includes_inline_comment():
    model = migjorn.parse(MODEL)
    c = model.cell(1)
    assert "$ fuel" in c.text  # inline comment is part of the card text
    assert "1 1 -1.0" in c.text
    # A surface card exposes its text too.
    assert model.surface(1).text.strip().startswith("1 SO 5")


def test_edit_material_and_density_in_place():
    model = migjorn.parse(MODEL)
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


def test_density_on_void_cell_raises():
    model = migjorn.parse(MODEL)
    # Cell 2 is void: it has no density field to set (assign a material first).
    try:
        model.cell(2).density = 1.0
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError setting density on a void cell")


def test_assign_material_to_void_cell():
    model = migjorn.parse(MODEL)
    cell = model.cell(2)  # "2 0 1 imp:n=0" (void)
    assert cell.is_void is True
    # Assigning a real material to a void cell just works: it gains a placeholder
    # density of 0.0 (no exception -- safe to do in a loop over cells).
    cell.material = 5
    assert cell.is_void is False
    assert cell.material == 5
    assert cell.density == 0.0
    assert "2 5 0 1 imp:n=0" in str(model)  # params tail preserved
    # Set the real density the usual way.
    cell.density = -2.0
    assert "2 5 -2 1 imp:n=0" in str(model)
    # The rest of the model is untouched and it re-parses cleanly.
    assert "1 1 -1.0 -1 2 #3 imp:n=1 $ fuel" in str(model)
    assert migjorn.parse(str(model)).diagnostics == []
    # Assigning material 0 makes it void again, dropping the density.
    cell.material = 0
    assert cell.is_void is True
    assert "2 0 1 imp:n=0" in str(model)


def test_edit_surface_coeffs_in_place():
    model = migjorn.parse(MODEL)
    s = model.surface(1)  # "1 SO 5"
    s.set_coeff(0, 7.5)
    assert model.surface(1).coeffs == [7.5]
    assert "1 SO 7.5" in str(model)
    # Whole-vector assignment (same length) also works.
    s.coeffs = [9.0]
    assert "1 SO 9" in str(model)
    # A wrong-length list is rejected (changing the count is structural).
    try:
        s.coeffs = [1.0, 2.0]
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for wrong coefficient count")
    # Out-of-range single-coefficient set is rejected too.
    try:
        s.set_coeff(9, 1.0)
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for out-of-range coefficient")
    # Everything else stays byte-identical and re-parses cleanly.
    assert "2 0 1 imp:n=0" in str(model)
    assert migjorn.parse(str(model)).diagnostics == []


def test_edit_surface_transform_field():
    src = "Xf demo\n1 0 -1\n\n1 3 SO 5\n2 SO 9\n\nm1 1001 1\ntr3 0 0 0\n"
    model = migjorn.parse(src)
    s1 = model.surface(1)
    assert s1.transform == 3
    s1.transform = 4
    assert "1 4 SO 5" in str(model)
    # Surface 2 has no transform field: adding one is now a lossless splice.
    model.surface(2).transform = 3
    assert "2 3 SO 9" in str(model)
    assert migjorn.parse(str(model)).diagnostics == []


def test_void_to_real_is_lossless():
    src = (
        "t\n1 0      -1 -2  $ hollow\n          imp:n=1\n"
        "\n1 SO 5\n2 PX 0\n\nm1 1001 1\n"
    )
    model = migjorn.parse(src)
    c = model.cell(1)
    # The two-step property flow (material, then density) is lossless.
    c.material = 200
    c.density = 2.2875
    out = str(model)
    # Only the header changes; the inline comment and continuation stay put.
    assert "1 200 2.2875      -1 -2  $ hollow" in out
    assert "          imp:n=1" in out
    assert model.cell(1).material == 200
    assert model.cell(1).density == 2.2875
    assert migjorn.parse(out).diagnostics == []


def test_add_and_remove_cell_param():
    src = "t\n1 1 -1.0 -1 imp:n=1  $ fuel\n\n1 SO 5\n\nm1 1001 1\n"
    model = migjorn.parse(src)
    c = model.cell(1)
    c.add_param("u=5")
    assert "1 1 -1.0 -1 imp:n=1 u=5  $ fuel" in str(model)
    assert c.remove_param("imp") is True
    assert "1 1 -1.0 -1 u=5  $ fuel" in str(model)
    assert c.remove_param("fill") is False
    assert migjorn.parse(str(model)).diagnostics == []


def test_edit_material_entries_in_place():
    model = migjorn.parse(MODEL)  # m1 1001.31c 0.667 8016.31c 0.333
    m = model.material(1)
    m.set_fraction(0, 0.7)
    m.set_zaid(1, "8016.70c")
    assert "m1 1001.31c 0.7 8016.70c 0.333" in str(model)
    assert model.material(1).entries[0] == ("1001.31c", 0.7)
    try:
        m.set_fraction(9, 1.0)
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for out-of-range entry")


def test_edit_transform_displacement_and_rotation():
    model = migjorn.parse(MODEL)  # tr1 0 0 5
    tr = model.transform(1)
    tr.displacement = (1.0, 2.0, 3.0)
    assert "tr1 1 2 3" in str(model)
    assert model.transform(1).displacement == (1.0, 2.0, 3.0)
    # With a full displacement, rotation entries can now be appended losslessly.
    tr.set_rotation([90.0])
    assert "tr1 1 2 3 90" in str(model)
    assert migjorn.parse(str(model)).diagnostics == []


def test_edit_transform_rotation_same_arity():
    src = "Rot demo\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\n*tr2 0 0 0 1 0 0 0 1 0 0 0 1\n"
    model = migjorn.parse(src)
    tr = model.transform(2)
    tr.set_rotation([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0])
    assert "0 0 0 2 0 0 0 2 0 0 0 2" in str(model)


def test_num_materials_and_transforms():
    model = migjorn.parse(MODEL)
    assert model.num_materials == 1
    assert model.num_transforms == 1


def test_add_and_remove_surface_in_geometry():
    model = migjorn.parse(MODEL)
    c = model.cell(1)  # "1 1 -1.0 -1 2 #3 imp:n=1 $ fuel"
    c.add_surface(-7)  # intersect with surface 7 (negative sense)
    assert -7 in c.signed_surfaces
    out = str(model)
    assert "-7" in out
    # The parameter tail (incl. inline comment) survives the restructure.
    assert "imp:n=1" in out
    # Other cards untouched, and the model still re-parses cleanly.
    assert "2 0 1 imp:n=0" in out
    assert migjorn.parse(out).diagnostics == []

    # Remove it again.
    assert c.remove_surface(7) is True
    assert -7 not in c.signed_surfaces


def test_add_and_remove_complement():
    model = migjorn.parse(MODEL)
    c = model.cell(2)  # "2 0 1 imp:n=0" (void)
    c.add_complement(1)
    assert 1 in c.cell_refs
    assert "#1" in str(model)
    assert c.remove_complement(1) is True
    assert 1 not in c.cell_refs


def test_geometry_edit_refused_when_it_would_empty():
    model = migjorn.parse(MODEL)
    c = model.cell(3)  # "3 0 -2 imp:n=1" — single surface
    try:
        c.remove_surface(2)
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError removing the last surface")


def test_add_cell_returns_editable_handle():
    model = migjorn.parse(MODEL)
    c = model.add_cell("10 0 -1")  # new void cell in the cell block
    assert c.id == 10 and c.is_void
    c.add_surface(2)  # edit the freshly-added cell like any other
    assert 2 in c.surface_ids
    out = str(model)
    assert "10 0 -1 2" in out
    # Existing cards untouched; model still re-parses cleanly.
    assert "1 1 -1.0 -1 2 #3 imp:n=1 $ fuel" in out
    assert model.num_cells == 4
    assert migjorn.parse(out).diagnostics == []


def test_add_surface_and_material():
    model = migjorn.parse(MODEL)
    s = model.add_surface("9 SO 12.0")
    assert s.id == 9 and s.kind == "SO" and s.coeffs == [12.0]
    m = model.add_material("m2 26000 -1.0")
    assert m.id == 2
    assert model.num_surfaces == 3
    assert migjorn.parse(str(model)).diagnostics == []


def test_remove_cell_and_validate():
    model = migjorn.parse(MODEL)
    # Cell 1 has a #3 complement; removing cell 3 makes it dangle.
    assert model.validate() == []
    assert model.remove_cell(3) is True
    assert model.num_cells == 2
    problems = model.validate()
    assert any("missing cell 3" in p for p in problems), problems
    # Removing a missing cell is a no-op.
    assert model.remove_cell(999) is False


def test_add_rejects_bad_text():
    model = migjorn.parse(MODEL)
    try:
        model.add_cell("this is not a cell")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for malformed cell text")


def test_renumber_materials_and_transforms():
    model = migjorn.parse(MODEL)
    model.renumber_materials({1: 101})
    out = str(model)
    assert "m101 1001.31c 0.667" in out  # Mn definition
    assert "1 101 -1.0" in out  # cell 1's material field
    # Transforms: the model has tr1 and surfaces have no transform field here,
    # so just check the definition renumbers.
    model.renumber_transforms(lambda n: n + 5)
    assert "tr6 0 0 5" in str(model)


def test_renumber_universes():
    src = (
        "Universe demo\n"
        "1 0 -1 u=5 imp:n=1\n"
        "2 0 -2 fill=5 imp:n=1\n\n"
        "1 SO 5\n2 SO 9\n\n"
        "m1 1001 1\n"
    )
    model = migjorn.parse(src)
    model.renumber_universes({5: 500})
    out = str(model)
    assert "u=500" in out and "fill=500" in out


def test_renumber_tallies_and_bins():
    src = (
        "Tally demo\n"
        "1 0 -1\n2 0 1\n\n"
        "1 SO 5\n\n"
        "m1 1001 1\n"
        "f4:n 1 2\n"  # cell-flux tally
        "e4 1 10\n"  # energy bins (companion)
    )
    model = migjorn.parse(src)
    # Renumbering cells updates the tally's cell bins.
    model.renumber_cells(lambda n: n + 10)
    assert "f4:n 11 12" in str(model)
    # renumber_tallies renames the tally id and its companion cards.
    model.renumber_tallies({4: 24})
    out = str(model)
    assert "f24:n" in out and "e24 1 10" in out


if __name__ == "__main__":
    test_parse_and_lossless()
    test_typed_access()
    test_renumber_offset_lossless_elsewhere()
    test_renumber_with_dict_and_callable()
    test_renumber_callable_error_propagates()
    test_card_text_includes_inline_comment()
    test_edit_material_and_density_in_place()
    test_density_on_void_cell_raises()
    test_assign_material_to_void_cell()
    test_edit_surface_coeffs_in_place()
    test_edit_surface_transform_field()
    test_edit_material_entries_in_place()
    test_edit_transform_displacement_and_rotation()
    test_edit_transform_rotation_same_arity()
    test_num_materials_and_transforms()
    test_add_and_remove_surface_in_geometry()
    test_add_and_remove_complement()
    test_geometry_edit_refused_when_it_would_empty()
    test_add_cell_returns_editable_handle()
    test_add_surface_and_material()
    test_remove_cell_and_validate()
    test_add_rejects_bad_text()
    test_renumber_materials_and_transforms()
    test_renumber_universes()
    test_renumber_tallies_and_bins()
    print("all migjorn binding smoke tests passed")
