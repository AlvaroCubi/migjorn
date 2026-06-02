"""Tests for the migjorn Python bindings (Model and card types)."""

import tempfile
from pathlib import Path

import migjorn
import pytest
from migjorn import Model

SMALL = Path(__file__).parent / "../../resources/simple_model.mcnp"


@pytest.fixture(scope="module")
def model() -> Model:
    return Model.from_file(str(SMALL))


# ── Model loading ─────────────────────────────────────────────────────────────


def test_model_from_file(model: Model):
    assert isinstance(model, Model)


def test_model_from_file_missing_raises():
    with pytest.raises(OSError):
        Model.from_file("/nonexistent/path.mcnp")


def test_model_with_new():
    with open(SMALL) as f:
        text = f.read()
    m = Model(text)
    assert isinstance(m, Model)


def test_model_loading_via_pathlib(model: Model):
    m = Model.from_file(SMALL)
    assert isinstance(m, Model)


def test_model_repr(model: Model):
    r = repr(model)
    assert r == "<Model cells=5 surfaces=6 data_cards=18>"


# ── Write round-trip ───────────────────────────────────────────────────────────


def test_write_to_file_round_trip(model: Model):
    with tempfile.NamedTemporaryFile(suffix=".mcnp", delete=False) as f:
        tmp = f.name

    try:
        model.write_to_file(tmp)
        m2 = Model.from_file(tmp)
        assert len(m2.cells) == len(model.cells)
        assert len(m2.surfaces) == len(model.surfaces)
        assert len(m2.materials) == len(model.materials)
        assert len(m2.transforms) == len(model.transforms)
    finally:
        Path(tmp).unlink(missing_ok=True)


def test_write_to_file_preserves_cell_ids(model: Model):
    with tempfile.NamedTemporaryFile(suffix=".mcnp", delete=False) as f:
        tmp = f.name

    try:
        model.write_to_file(tmp)
        m2 = Model.from_file(tmp)
        original_ids = sorted(c.card_id for c in model.cells)
        written_ids = sorted(c.card_id for c in m2.cells)
        assert original_ids == written_ids
    finally:
        Path(tmp).unlink(missing_ok=True)


def test_write_to_file_preserves_mutations(model: Model):
    m = Model.from_file(str(SMALL))
    m.cells[0].card_id = 999999
    m.surfaces[0].card_id = 888888

    with tempfile.NamedTemporaryFile(suffix=".mcnp", delete=False) as f:
        tmp = f.name
    try:
        m.write_to_file(tmp)
        m2 = Model.from_file(tmp)
        assert any(c.card_id == 999999 for c in m2.cells)
        assert any(s.card_id == 888888 for s in m2.surfaces)
    finally:
        Path(tmp).unlink(missing_ok=True)


def test_write_to_file_invalid_path_raises():
    m = Model.from_file(str(SMALL))
    with pytest.raises(OSError):
        m.write_to_file("/nonexistent_dir/output.mcnp")


def test_deepcopy():
    from copy import deepcopy

    m = Model.from_file(str(SMALL))
    copied_m = deepcopy(m)
    assert isinstance(copied_m, Model)

    # Changing the copied model does not affect the original
    copied_m.cells[0].card_id = 999999
    assert m.cells[0].card_id != 999999


# ── Public API surface ─────────────────────────────────────────────────────────


def test_public_names_importable():
    """Ensures all names re-exported from __init__.py are accessible."""
    assert hasattr(migjorn, "Model")
    assert hasattr(migjorn, "CellCard")
    assert hasattr(migjorn, "SurfaceCard")
    assert hasattr(migjorn, "MaterialCard")
    assert hasattr(migjorn, "Zaid")
    assert hasattr(migjorn, "TransformCard")
    assert hasattr(migjorn, "UnparsedCard")
    assert hasattr(migjorn, "run")
