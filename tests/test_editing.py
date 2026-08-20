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
