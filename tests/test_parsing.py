"""Correct-parsing contract: assert the pinned facts in ``expectations.toml``.

Where ``test_roundtrip.py`` proves we preserve the *bytes*, this proves we
understand the *structure* — cell/surface counts, materials, densities,
geometry references, parameters. Together they are the parser's acceptance test.

Run: ``pytest tests/test_parsing.py -q``.
"""

from __future__ import annotations

from pathlib import Path

import migjorn
import pytest
import tomllib

HERE = Path(__file__).parent
CORPUS = HERE / "corpus"
EXPECT = tomllib.loads((HERE / "expectations.toml").read_text())


@pytest.fixture(scope="module")
def models() -> dict[str, migjorn.Model]:
    return {
        name: migjorn.parse((CORPUS / f"{name}.mcnp").read_text(encoding="utf-8"))
        for name in EXPECT
    }


@pytest.mark.parametrize("name", list(EXPECT))
def test_counts(models, name: str) -> None:
    model, exp = models[name], EXPECT[name]
    if "num_cells" in exp:
        assert model.num_cells == exp["num_cells"], "cell count"
    if "num_surfaces" in exp:
        assert model.num_surfaces == exp["num_surfaces"], "surface count"
    if "num_materials" in exp:
        assert model.num_materials == exp["num_materials"], "material count"
    if "num_transforms" in exp:
        assert model.num_transforms == exp["num_transforms"], "transform count"


# NOTE on shape: `cell.1 = {...}` in TOML is a *nested* table, so these arrive
# as EXPECT[name]["cell"]["1"] — not as a flat "cell.1" key. Iterating the
# section's top-level keys looking for a "cell." prefix silently matches nothing
# and asserts nothing, which is why this reads the nested tables directly.


@pytest.mark.parametrize("name", list(EXPECT))
def test_cells(models, name: str) -> None:
    model, exp = models[name], EXPECT[name]
    for cid, want in exp.get("cell", {}).items():
        cell = model.cell(int(cid))
        assert cell is not None, f"{name}: cell {cid} not found"
        _check_cell(cell, want)


@pytest.mark.parametrize("name", list(EXPECT))
def test_surfaces(models, name: str) -> None:
    model, exp = models[name], EXPECT[name]
    for sid, want in exp.get("surface", {}).items():
        surf = model.surface(int(sid))
        assert surf is not None, f"{name}: surface {sid} not found"
        _check_surface(surf, want)


@pytest.mark.parametrize("name", list(EXPECT))
def test_materials_and_transforms(models, name: str) -> None:
    model, exp = models[name], EXPECT[name]
    for mid, want in exp.get("material", {}).items():
        mat = model.material(int(mid))
        assert mat is not None, f"{name}: material {mid} not found"
        got = [[z, f] for (z, f) in mat.entries]
        assert got == want["entries"], f"{name}: material {mid} entries"
    for tid, want in exp.get("transform", {}).items():
        tr = model.transform(int(tid))
        assert tr is not None, f"{name}: transform {tid} not found"
        if "degrees" in want:
            assert tr.degrees == want["degrees"], f"{name}: transform {tid} degrees"
        if "displacement" in want:
            assert list(tr.displacement) == want["displacement"]


def _check_cell(cell: migjorn.Cell, want: dict) -> None:
    if "material" in want:
        assert cell.material == want["material"]
    if "density" in want:
        assert cell.density == pytest.approx(want["density"])
    if "void" in want:
        assert cell.is_void == want["void"]
    if "signed_surfaces" in want:
        assert cell.signed_surfaces == want["signed_surfaces"]
    if "surface_ids" in want:
        assert sorted(set(cell.surface_ids)) == sorted(set(want["surface_ids"]))
    if "cell_refs" in want:
        assert sorted(set(cell.cell_refs)) == sorted(set(want["cell_refs"]))
    if "like" in want:
        assert cell.like == want["like"]
    if "universe" in want:
        assert cell.universe == want["universe"]
    if "lattice" in want:
        assert (cell.param("lat") is not None) == want["lattice"]
    if "imp_n" in want:
        assert cell.param("imp:n").value == want["imp_n"]  # type: ignore
    if "imp_p" in want:
        assert cell.param("imp:p").value == want["imp_p"]  # type: ignore
    if "vol" in want:
        assert cell.param("vol").value == want["vol"]  # type: ignore
    if "fill_universe" in want:
        assert cell.fill.universe == want["fill_universe"]  # type: ignore
    if "fill_starred" in want:
        assert cell.fill.starred == want["fill_starred"]  # type: ignore


def _check_surface(surf: migjorn.Surface, want: dict) -> None:
    if "kind" in want:
        assert surf.kind == want["kind"]
    if "coeffs" in want:
        assert list(surf.coeffs) == pytest.approx(want["coeffs"])
    if "reflective" in want:
        assert surf.reflective == want["reflective"]
    if "white" in want:
        assert surf.white == want["white"]
    if "transform" in want:
        assert surf.transform == want["transform"]
