from pathlib import Path

import pytest
from migjorn import MaterialCard, Model, TallyCard, TransformCard, UnparsedCard, Zaid

SMALL = Path(__file__).parent / "../../resources/simple_model.mcnp"


@pytest.fixture(scope="module")
def model() -> Model:
    return Model.from_file(str(SMALL))


def test_model_materials_count(model: Model):
    assert len(model.materials) == 2


def test_material_type(model: Model):
    assert isinstance(model.materials[0], MaterialCard)


def test_material_ids(model: Model):
    ids = {m.material_id for m in model.materials}
    assert ids == {100, 400}


def test_material_zaids_type(model: Model):
    mat = next(m for m in model.materials if m.material_id == 400)
    zaids = mat.zaids
    assert isinstance(zaids, list)
    assert all(isinstance(z, Zaid) for z in zaids)


def test_material_zaids_count(model: Model):
    mat400 = next(m for m in model.materials if m.material_id == 400)
    assert len(mat400.zaids) == 5

    mat100 = next(m for m in model.materials if m.material_id == 100)
    assert len(mat100.zaids) == 3


def test_material_zaid_fields(model: Model):
    mat = next(m for m in model.materials if m.material_id == 400)
    z = mat.zaids[0]
    assert z.isotope == 1001
    assert z.library == "32c"
    assert z.fraction == pytest.approx(0.063391)


def test_material_repr(model: Model):
    mat = next(m for m in model.materials if m.material_id == 400)
    assert repr(mat) == "<MaterialCard id=400 zaids=5>"


def test_material_generics(model: Model):
    mat = next(m for m in model.materials if m.material_id == 400)
    assert mat.material_id == 400
    mat.card_id = 500
    assert mat.updated_text == mat.original_text.replace("M400", "M500")


# ── Transforms ────────────────────────────────────────────────────────────────


def test_model_transforms_count(model: Model):
    assert len(model.transforms) > 0


def test_transform_type(model: Model):
    assert isinstance(model.transforms[0], TransformCard)


def test_transform_fields(model: Model):
    t = model.transforms[0]
    assert t.transform_id == 1
    assert t.starred is True
    assert isinstance(t.coeffs, list)
    assert t.coeffs[:3] == pytest.approx([0.0, 0.0, 0.0])


def test_transform_repr(model: Model):
    assert repr(model.transforms[0]) == "<TransformCard id=1>"


def test_transform_generics(model: Model):
    t = model.transforms[0]
    assert t.card_id == 1
    t.card_id = 42
    assert t.updated_text == t.original_text.replace("TR1", "TR42")


# ── Tally cards ───────────────────────────────────────────────────────────


def test_model_tally_cards_count(model: Model):
    assert len(model.tallies) == 2


def test_tally_card_type(model: Model):
    assert isinstance(model.tallies[0], TallyCard)


def test_tally_generics(model: Model):
    t = model.tallies[0]
    assert t.tally_id == 4
    assert t.card_id == 4
    t.tally_id = 10
    assert t.updated_text == t.original_text.replace("F4", "F10")


# ── Other data cards ───────────────────────────────────────────────────────────


def test_other_data_cards_count(model: Model):
    assert len(model.unparsed_data_cards) == 10


def test_other_card_type(model: Model):
    assert isinstance(model.unparsed_data_cards[0], UnparsedCard)


def test_other_card_names(model: Model):
    names = {o.card_name for o in model.unparsed_data_cards}
    assert "RAND" in names
    assert "NPS" in names
    assert "PRDMP" in names


def test_other_card_repr(model: Model):
    others = {o.card_name: o for o in model.unparsed_data_cards}
    assert repr(others["NPS"]) == "<UnparsedCard NPS>"


def test_other_card_generics(model: Model):
    o = next(o for o in model.unparsed_data_cards if o.card_name == "NPS")
    assert o.card_id == "NPS"
    assert "NPS    1E9" in o.updated_text
