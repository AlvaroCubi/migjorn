from pathlib import Path

from migjorn import Model

SMALL = Path(__file__).parent / "../../resources/simple_model.mcnp"


def test_renumber_cells():
    model = Model.from_file(str(SMALL))
    model.renumber_cells(offset=1000)
    assert model.cells[0].card_id == 1001
    assert model.cells[-1].card_id == 1005


def test_renumber_cells_with_range():
    model = Model.from_file(str(SMALL))
    model.renumber_cells(offset=1000, range=(1, 3))
    assert model.cells[0].card_id == 1001
    assert model.cells[2].card_id == 1003
    assert model.cells[3].card_id == 4


def test_renumber_surfaces():
    model = Model.from_file(str(SMALL))
    model.renumber_surfaces(offset=1000)
    assert model.surfaces[0].card_id == 1010
    assert model.surfaces[-1].card_id == 1015


def test_renumber_surfaces_with_range():
    model = Model.from_file(str(SMALL))
    model.renumber_surfaces(offset=1000, range=(10, 13))
    assert model.surfaces[0].card_id == 1010
    assert model.surfaces[3].card_id == 1013
    assert model.surfaces[4].card_id == 14


def test_renumber_materials():
    model = Model.from_file(str(SMALL))
    model.renumber_materials(offset=-50)
    assert model.materials[0].material_id == 350
    assert model.materials[-1].material_id == 50


def test_renumber_materials_with_range():
    model = Model.from_file(str(SMALL))
    model.renumber_materials(offset=1000, range=(400, 400))
    assert model.materials[0].material_id == 1400
    assert model.materials[-1].material_id == 100


def test_renumber_transformations():
    model = Model.from_file(str(SMALL))
    model.renumber_transformations(offset=1000)
    assert model.transforms[0].transform_id == 1001
    assert model.transforms[-1].transform_id == 1010


def test_renumber_transformations_with_range():
    model = Model.from_file(str(SMALL))
    model.renumber_transformations(offset=1000, range=(1, 3))
    assert model.transforms[0].transform_id == 1001
    assert model.transforms[2].transform_id == 1003
    assert model.transforms[-1].transform_id == 10


def test_renumber_universes():
    model = Model.from_file(str(SMALL))
    model.renumber_universes(offset=1000)
    universe_param = next(p for p in model.cells[0].params if p.param_type == "U")
    assert universe_param.value == 1001


def test_renumber_surfaces_updates_cell_geometry():
    """Surface renumbering must propagate into cell geometry references."""
    model = Model.from_file(str(SMALL))
    model.renumber_surfaces(offset=1000)
    # Cell 1 originally references surfaces -10, -11, 12; all should shift to 1010/1011/1012
    geo = model.cells[0].geometry
    assert "-1010" in geo
    assert "-1011" in geo
    assert "1012" in geo


def test_renumber_materials_updates_cell_material_id():
    """Material renumbering must update the material ID reference in cells."""
    model = Model.from_file(str(SMALL))
    # Cell 0 references M100; shift M100 → M1100
    model.renumber_materials(offset=1000)
    assert model.cells[0].material_id == 1100


def test_renumber_cells_updates_complement_geometry():
    """Cell renumbering must update #N complement references in other cells."""
    # Construct a model that has a complement reference #1
    text = "Title\n1 1 -1 -10 IMP:N=1\n2 0 #1 IMP:N=0\n\n10 PX 0\n\nM1 1001.32c 1.0\n"
    model = Model(text)
    model.renumber_cells(offset=100)
    # Cell 1 → 101, so the complement #1 in cell 2's geometry should become #101
    geo = model.cells[1].geometry
    assert "101" in geo
    assert "1" not in [
        g for g in geo if g.lstrip("-").isdigit() and int(g.lstrip("-")) == 1
    ]
