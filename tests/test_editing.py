"""Editing-semantics contract, including the no-whole-file-reparse guarantee.

These tests are the behavioural spec for the iterative-editing workload migjorn
is built around: many small adds / removes / value edits interleaved
with reads, all staying cheap and lossless. The performance *thresholds* live in
``benches/`` (they need a large input); here we pin the *semantics* that must
hold at any size.

Run: ``pytest tests/test_editing.py -q``.
"""

from __future__ import annotations

from pathlib import Path

import migjorn
import pytest

CORPUS = Path(__file__).parent / "corpus"


@pytest.fixture
def model() -> migjorn.Model:
    return migjorn.parse((CORPUS / "minimal.mcnp").read_text(encoding="utf-8"))


# --- structural edits -------------------------------------------------------


def test_add_cell_appends_and_is_visible(model: migjorn.Model) -> None:
    before = model.num_cells
    cell = model.add_cell("3 1 -1.0 -1 imp:n=1")
    assert model.num_cells == before + 1
    assert cell.id == 3
    assert model.cell(3) is not None
    assert "3 1 -1.0 -1 imp:n=1" in model.to_source()


def test_remove_cell(model: migjorn.Model) -> None:
    assert model.remove_cell(2) is True
    assert model.cell(2) is None
    assert model.num_cells == 1
    assert model.remove_cell(999) is False  # absent id


def test_add_then_remove_is_identity(model: migjorn.Model) -> None:
    src = model.to_source()
    model.add_cell("42 0 -1 imp:n=1")
    model.remove_cell(42)
    assert model.to_source() == src


def test_interleaved_edits_and_reads_stay_consistent(model: migjorn.Model) -> None:
    """A read between two edits must never observe stale state (the class of bug
    the old splice/materialize design had to guard against by hand)."""
    model.add_cell("3 0 -1 imp:n=1")
    cell = model.cell(3)
    assert cell is not None
    assert cell.is_void
    cell.material = 1  # read-modify
    assert cell.material == 1  # read-after-write, no explicit flush
    model.remove_cell(3)
    assert model.cell(3) is None


# --- title -------------------------------------------------------------------


def test_title_getter_reads_the_title(model: migjorn.Model) -> None:
    assert model.title == "Minimal valid model"


def test_title_getter_is_none_when_absent() -> None:
    assert migjorn.parse("").title is None


def test_title_setter_replaces_existing(model: migjorn.Model) -> None:
    model.title = "New Title"
    assert model.title == "New Title"
    assert model.to_source().startswith("New Title\n")


def test_title_setter_creates_when_absent() -> None:
    m = migjorn.parse("")
    m.title = "Fresh"
    assert m.title == "Fresh"


# --- id setters ---------------------------------------------------------


def test_cell_clone_workflow_via_add_cell_and_id_setter() -> None:
    m = migjorn.parse("t\n1 1 -1.0 -1 imp:n=1 vol=3\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n")
    source = m.cell(1)
    assert source is not None
    clone = m.add_cell(source.text)
    clone.id = 501
    assert clone.id == 501
    assert clone.material == 1
    assert clone.density == -1.0
    # the original is untouched, and both cards are present
    assert m.cell(1) is not None
    out = m.to_source()
    assert "1 1 -1.0 -1 imp:n=1 vol=3" in out
    assert "501 1 -1.0 -1 imp:n=1 vol=3" in out
    # the clone diverges independently from here
    clone.geometry_text = "-2"
    assert clone.geometry_text == "-2"
    assert source.geometry_text == "-1"


def test_cell_id_setter_leaves_existing_references_dangling() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n2 0 1 #1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    cell.id = 501
    assert m.cell(1) is None
    assert m.cell(501) is not None
    other = m.cell(2)
    assert other is not None
    assert other.cell_refs == [1]  # not moved — this is not a rename
    assert m.validate() != []


def test_surface_id_setter() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    surface = m.surface(1)
    assert surface is not None
    surface.id = 501
    assert surface.id == 501
    assert m.surface(1) is None
    assert m.surface(501) is not None


def test_material_id_setter() -> None:
    m = migjorn.parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    material = m.material(1)
    assert material is not None
    material.id = 501
    assert material.id == 501
    assert m.material(1) is None
    assert m.material(501) is not None


def test_transform_id_setter() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n")
    transform = m.transform(1)
    assert transform is not None
    transform.id = 501
    assert transform.id == 501
    assert m.transform(1) is None
    assert m.transform(501) is not None


# --- in-card value edits ----------------------------------------------------


def test_set_material_crosses_void_boundary(model: migjorn.Model) -> None:
    cell = model.cell(2)
    assert cell is not None
    cell.material = 5  # void → real, density field appears
    assert cell.material == 5
    assert not cell.is_void
    cell.material = 0  # back to void, density dropped
    assert cell.is_void
    assert cell.density is None


def test_cell_param_edit_preserves_rest_of_card() -> None:
    m = migjorn.parse("t\n1 1 -1.0 -1 imp:n=1 vol=3 $ keep\n\n1 SO 5\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    cell.set_param("imp:n", "2")
    out = m.to_source()
    assert "imp:n=2" in out
    assert "vol=3" in out
    assert "$ keep" in out


def test_geometry_reads_terms_in_file_order() -> None:
    m = migjorn.parse(
        "t\n1 0 -1 2 #3\n2 0 1 imp:n=1\n3 0 1 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n"
    )
    cell = m.cell(1)
    assert cell is not None
    terms = cell.geometry
    assert [t.kind for t in terms] == ["surface", "surface", "complement"]
    assert [t.text for t in terms] == ["-1", "2", "#3"]


def test_geometry_term_repr_and_str() -> None:
    m = migjorn.parse("t\n1 0 -1 #2\n2 0 1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    surface, complement = cell.geometry
    assert str(surface) == "-1"
    assert repr(surface) == 'GeometryTerm(kind="surface", text="-1")'
    assert str(complement) == "#2"
    # round-tripping through str() reconstructs a readable expression
    assert " ".join(str(t) for t in cell.geometry) == "-1 #2"


def test_geometry_text_reads_the_exact_source() -> None:
    m = migjorn.parse(
        "t\n1 0 -1  2   #3\n2 0 1 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n"
    )
    cell = m.cell(1)
    assert cell is not None
    assert cell.geometry_text == "-1  2   #3"


def test_geometry_text_setter_replaces_the_whole_expression() -> None:
    m = migjorn.parse(
        "t\n1 0 -1 imp:n=1 vol=3\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n"
    )
    cell = m.cell(1)
    assert cell is not None
    cell.geometry_text = "(1 2) : 3"
    assert cell.geometry_text == "(1 2) : 3"
    assert cell.signed_surfaces == [1, 2, 3]
    out = m.to_source()
    assert "1 0 (1 2) : 3 imp:n=1 vol=3" in out


def test_geometry_text_unites_several_cells() -> None:
    m = migjorn.parse(
        "t\n1 0 -1 -2 imp:n=1\n2 0 -3 imp:n=1\n3 0 -4 imp:n=1\n"
        "4 0 1 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n4 SO 8\n\nm1 1001 1\n"
    )
    to_unite = [m.cell(1), m.cell(2), m.cell(3)]
    target = m.cell(4)
    assert target is not None
    target.geometry_text = " : ".join(f"({c.geometry_text})" for c in to_unite)
    assert target.geometry_text == "(-1 -2) : (-3) : (-4)"
    assert target.signed_surfaces == [-1, -2, -3, -4]


def test_set_geometry_term_substitutes_a_surface_across_cells() -> None:
    m = migjorn.parse("t\n1 0 -1 2\n2 0 -2 3\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n")
    # a generic "find surface 2, replace with 4" pass over every cell — the
    # kind of ad-hoc analysis the geometry API is meant to make possible
    # without a dedicated method for every case.
    for cell in m.cells():
        for position, term in enumerate(cell.geometry):
            if term.kind == "surface" and term.text == "2":
                cell.set_geometry_term(position, "4")
    assert m.cell(1).surface_ids == [1, 4]
    assert m.cell(2).surface_ids == [2, 3]  # -2 keeps its sense, untouched


def test_set_geometry_term_accepts_a_negative_index() -> None:
    m = migjorn.parse("t\n1 0 -1 2 -3\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    cell.set_geometry_term(-2, "123")
    assert cell.signed_surfaces == [-1, 123, -3]


def test_set_geometry_term_negative_index_past_the_start_raises_value_error() -> None:
    m = migjorn.parse("t\n1 0 -1 2\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    with pytest.raises(ValueError):
        cell.set_geometry_term(-3, "5")


def test_set_geometry_term_out_of_range_raises_value_error() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    with pytest.raises(ValueError):
        cell.set_geometry_term(99, "5")


def test_insert_geometry_term_hashes_a_cell_with_another() -> None:
    m = migjorn.parse(
        "t\n1 0 1 2 3 imp:n=1\n2 0 -1 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n"
    )
    cell = m.cell(1)
    assert cell is not None
    n = len(cell.geometry)  # read once, before any insert
    cell.insert_geometry_term(0, "(")
    cell.insert_geometry_term(n + 1, ")")
    cell.insert_geometry_term(n + 2, "#2")
    assert "1 0 ( 1 2 3 ) #2 imp:n=1" in m.to_source()
    assert cell.cell_refs == [2]


def test_insert_geometry_term_appends_when_position_equals_length() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    cell.insert_geometry_term(len(cell.geometry), ": 2")
    assert cell.signed_surfaces == [-1, 2]


def test_insert_geometry_term_past_the_end_raises_value_error() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    cell = m.cell(1)
    assert cell is not None
    with pytest.raises(ValueError):
        cell.insert_geometry_term(len(cell.geometry) + 1, "2")


# --- renumbering ------------------------------------------------------------


def test_renumber_surfaces_updates_defs_and_refs(model: migjorn.Model) -> None:
    model.offset_surfaces(100)
    out = model.to_source()
    assert "101 SO 5" in out  # definition
    assert "-101 imp:n=1" in out  # reference in cell 1, sense preserved


def test_renumber_cells_updates_defs_and_complements() -> None:
    m = migjorn.parse("t\n1 0 -1 imp:n=1\n2 0 1 #1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    m.renumber_cells({1: 501})
    out = m.to_source()
    assert "501 0 -1" in out
    assert "#501" in out


def test_renumber_is_otherwise_lossless(model: migjorn.Model) -> None:
    src = model.to_source()
    model.renumber_surfaces({})  # empty mapping → no visible change
    assert model.to_source() == src
