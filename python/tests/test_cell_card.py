# ruff: noqa: PLR2004
from pathlib import Path

import pytest
from migjorn import CellCard, CellParameter, Model

SMALL = Path(__file__).parent / "../../resources/simple_model.mcnp"


@pytest.fixture(scope="module")
def model() -> Model:
    return Model.from_file(str(SMALL))


def test_model_cells_count(model: Model):
    assert len(model.cells) == 5


def test_cell_type(model: Model):
    assert isinstance(model.cells[0], CellCard)


def test_cell_ids_in_expected_range(model: Model):
    ids = {c.card_id for c in model.cells}
    assert min(ids) == 1
    assert max(ids) == 5


def test_cell_material_id(model: Model):
    cell = model.cells[0]
    assert cell.card_id == 1
    assert cell.material_id == 100


def test_cell_density(model: Model):
    assert model.cells[0].density == pytest.approx(-7.93)


def test_void_cell_has_no_density(model: Model):
    void_cell = next(c for c in model.cells if c.material_id == 0)
    assert void_cell.card_id == 3
    assert void_cell.density is None


def test_cell_geometry_is_list_of_strings(model: Model):
    geo = model.cells[0].geometry
    assert isinstance(geo, list)
    assert all(isinstance(g, str) for g in geo)


def test_cell_geometry_contents(model: Model):
    geo = model.cells[0].geometry
    # Expected: ['(', '-10', '-11', '12', ')']
    assert geo[0] == "("
    assert "12" in geo
    assert "-10" in geo


def _param(cell, param_type: str) -> CellParameter | None:
    """Helper: find the first parameter matching param_type, or None."""
    return next((p for p in cell.params if p.param_type == param_type), None)


def test_cell_imp_n_and_imp_p(model: Model):
    cell = model.cells[0]
    assert _param(cell, "IMP:N").value == pytest.approx(1.0)  # type: ignore
    assert _param(cell, "IMP:P").value == pytest.approx(1.0)  # type: ignore


def test_cell_universe(model: Model):
    assert _param(model.cells[0], "U").value == 1  # type: ignore


def test_cell_optional_fields_default_to_none(model: Model):
    cell = model.cells[0]
    assert _param(cell, "VOL") is None
    assert _param(cell, "LAT") is None
    assert _param(cell, "FILL") is None


def test_cell_repr(model: Model):
    assert repr(model.cells[0]) == "<CellCard id=1 mat=100>"


def test_cell_mutation_is_visible():
    """Mutating a cell is visible through subsequent property access."""
    # Use a fresh model to avoid polluting the shared fixture
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original_id = cell.card_id
    cell.card_id = 999999
    assert m.cells[0].card_id == 999999
    # Restore so other tests aren't affected if fixture were shared
    cell.card_id = original_id


def test_cell_density_mutation():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.density = -5.0
    assert m.cells[0].density == pytest.approx(-5.0)


def test_void_cell_set_density():
    m = Model.from_file(str(SMALL))
    non_void_cell = next(c for c in m.cells if c.material_id != 0)
    non_void_cell.density = -5.0
    assert non_void_cell.density == pytest.approx(-5.0)


def test_cell_imp_n_mutation():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    _param(cell, "IMP:N").value = 0.5  # type: ignore
    assert _param(m.cells[0], "IMP:N").value == pytest.approx(0.5)  # type: ignore


def test_missing_param_returns_none(model: Model):
    """_param() returns None for an absent keyword; accessing .value on it raises AttributeError."""
    cell = model.cells[0]
    missing = _param(cell, "FILL")  # cell 1 has no FILL param
    assert missing is None
    with pytest.raises(AttributeError):
        _ = missing.value  # type: ignore


def normalize_line_endings(s: str) -> str:
    return s.replace("\r\n", "\n").replace("\r", "\n").strip()


def test_cell_get_original_text(model: Model):
    cell = model.cells[2]
    original_text = normalize_line_endings(cell.original_text)
    expected = normalize_line_endings("3  0         (13 -14)      IMP:N=1 IMP:P=1")

    assert original_text == expected


def test_cell_get_updated_text_after_mutation():
    m = Model.from_file(str(SMALL))
    cell = m.cells[2]
    cell.card_id = 999999
    cell.material_id = 2
    cell.density = -5.0
    _param(cell, "IMP:N").value = 0.5  # type: ignore

    updated_text = normalize_line_endings(cell.updated_text)
    expected = normalize_line_endings(
        "999999  2 -5         (13 -14)      IMP:N=0.5 IMP:P=1"
    )

    assert updated_text == expected


def test_void_cell_method():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.void_cell()
    assert cell.material_id == 0
    assert cell.density is None


def test_insert_parameter_appends_by_default():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original_count = len(cell.params)
    cell.insert_parameter("VOL=42.5")
    assert len(cell.params) == original_count + 1
    last = cell.params[-1]
    assert last.param_type == "VOL"
    assert last.value == pytest.approx(42.5)


def test_insert_parameter_at_index():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.insert_parameter("U=99", index=0)
    first = cell.params[0]
    assert first.param_type == "U"
    assert first.value == 99


def test_insert_parameter_case_insensitive():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.insert_parameter("imp:n=0.5")
    last = cell.params[-1]
    assert last.param_type == "IMP:N"
    assert last.value == pytest.approx(0.5)


def test_other_parameter_value_can_be_mutated():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.insert_parameter("FOO=bar")

    other = cell.params[-1]
    assert "FOO=bar" in other.param_type
    assert "FOO=bar" in other.value  # type: ignore

    other.value = "BAR=2"  # type: ignore
    assert other.value == "BAR=2"
    assert other.param_type == "BAR=2"

    expected_text = normalize_line_endings(
        "1  100 -7.93 (-10 -11 12) IMP:N=1 IMP:P=1 U=1 BAR=2"
    )
    assert normalize_line_endings(cell.updated_text) == expected_text


def test_insert_parameter_visible_in_updated_text():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.insert_parameter("VOL=1.5")
    assert "VOL=" in cell.updated_text


def test_remove_geometry_element_shrinks_list():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original_len = len(cell.geometry)
    cell.remove_geometry_element(0)
    assert len(cell.geometry) == original_len - 1


def test_remove_geometry_element_preserves_roundtrip():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original = cell.original_text
    cell.remove_geometry_element(0)
    updated = cell.updated_text
    # Removing the first token must shorten the text but keep the rest intact
    assert len(updated) < len(original)
    for token in cell.geometry:
        assert token in updated


def test_insert_geometry_element_appends_surface():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original_len = len(cell.geometry)
    cell.insert_geometry_elements("-999")
    assert len(cell.geometry) == original_len + 1
    assert cell.geometry[-1] == "-999"
    assert "-999" in cell.updated_text


def test_insert_geometry_element_at_index():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    cell.insert_geometry_elements(":", 0)
    assert cell.geometry[0] == ":"


def test_insert_geometry_element_complement_pair():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    original_len = len(cell.geometry)
    # "#99" should insert two elements: complement operator + cell reference
    cell.insert_geometry_elements("#99")
    assert len(cell.geometry) == original_len + 2
    assert cell.geometry[-2] == "#"
    assert cell.geometry[-1] == "99"


def test_insert_geometry_element_invalid_raises():
    m = Model.from_file(str(SMALL))
    cell = m.cells[0]
    with pytest.raises(ValueError):
        cell.insert_geometry_elements("IMP:N=1")
