from pathlib import Path

import pytest
from migjorn import Model, SurfaceCard

SMALL = Path(__file__).parent / "../../resources/simple_model.mcnp"


@pytest.fixture(scope="module")
def model() -> Model:
    return Model.from_file(str(SMALL))


def test_model_surfaces_count(model: Model):
    assert len(model.surfaces) == 6


def test_surface_type(model: Model):
    assert isinstance(model.surfaces[0], SurfaceCard)


def test_surface_ids_in_expected_range(model: Model):
    ids = {s.card_id for s in model.surfaces}
    assert min(ids) == 10
    assert max(ids) == 15


def test_surface_fields(model: Model):
    s = model.surfaces[0]
    assert s.card_id == 10
    assert s.surface_type == "KZ"
    assert s.transform_id is None
    assert s.coeffs == pytest.approx([-5.0, 0.5, 1.0])


def test_surface_coeffs_is_list_of_floats(model: Model):
    for s in model.surfaces:
        assert isinstance(s.coeffs, list)
        assert all(isinstance(v, float) for v in s.coeffs)


def test_surface_repr(model: Model):
    assert repr(model.surfaces[0]) == "<SurfaceCard id=10 type=KZ>"


def test_surface_id_mutation(model: Model):
    m = Model.from_file(str(SMALL))
    surf = m.surfaces[0]
    surf.card_id = 999999
    assert m.surfaces[0].card_id == 999999


def test_surface_transform_id_setter():
    # Build a minimal model from text that has a surface with transform 1
    text = "Title\n1 0 -10 IMP:N=1\n2 0 10 IMP:N=0\n\n10 1 PX 0\n\nTR1 1 0 0\n"
    m = Model(text)
    surf = m.surfaces[0]
    assert surf.transform_id == 1
    surf.card_id = 20
    assert surf.card_id == 20
    assert "20" in surf.updated_text
