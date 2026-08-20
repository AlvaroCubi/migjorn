"""Tests for the PyO3 boundary itself.

The top-level ``tests/`` suite (parsing, round-trip, editing) exercises the
public API surface, but does so through calls that happen to succeed. Nothing
there proves what happens at the FFI boundary when things go wrong or when
handles outlive the state they point at. This suite covers that layer
specifically:

- exception mapping (which Rust error becomes which Python exception, and
  with what message)
- live-handle semantics: a handle into a removed card must raise, not read
  garbage or panic across the boundary
- diagnostics content surviving the boundary intact
- the composition API (validate / merge / extract_universe / extract_level0 /
  extract_cells) and the per-field mutators (set_coeff(s), set_fraction,
  set_zaid, ...)

Run against a build of *this* checkout's bindings, e.g.:
    cd crates/migjorn-py && maturin build --release --out ../../dist
    pip install --force-reinstall --no-deps ../../dist/migjorn-*.whl
    pytest crates/migjorn-py/tests -q
"""

from __future__ import annotations

import migjorn
import pytest

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

MALFORMED = """\
Malformed input the parser must recover from (never panic)
1 1 -1.0 -1 imp:n=1
2 0 1 -              $ dangling geometry operator
3 0 ((1 2 imp:n=1   $ unbalanced parentheses
4 9 -1.0 imp:n=1    $ material with no geometry surfaces

1 SO 5
2 PX
3 GQ 1 1 1          $ too few coefficients

m1 1001 zzz         $ non-numeric fraction
mt99
"""


# --- exception mapping -------------------------------------------------------


def test_no_such_field_raises_value_error() -> None:
    # cell 2 is void: no density field exists to set.
    model = migjorn.parse(MODEL)
    with pytest.raises(ValueError, match="does not exist on this card"):
        cell = model.cell(2)
        assert cell is not None
        cell.density = -2.0


def test_add_card_with_no_matching_block_raises_value_error() -> None:
    # A model with no data block at all: nowhere to append a material.
    model = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n")
    with pytest.raises(ValueError, match="no block of that kind"):
        model.add_material("m1 1001 1")


def test_merge_error_reports_every_conflicting_id() -> None:
    a = migjorn.parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    b = migjorn.parse("t\n1 0 -2 imp:n=1\n\n1 SO 6\n\nm2 8016 1\n")
    assert issubclass(migjorn.MergeError, ValueError)
    with pytest.raises(migjorn.MergeError) as exc:
        a.merge([b])
    # both the shared cell id and the shared surface id must be named
    assert "cell" in str(exc.value)
    assert "surface" in str(exc.value)
    # a failed merge must not have partially applied
    assert a.num_cells == 1
    assert a.num_surfaces == 1


# --- removed-handle semantics -------------------------------------------------


@pytest.mark.parametrize(
    "get_handle",
    [
        lambda m: m.cell(2),
        lambda m: m.surface(2),
    ],
)
def test_using_a_removed_handle_raises_value_error(get_handle) -> None:
    model = migjorn.parse(MODEL)
    handle = get_handle(model)
    if isinstance(handle, migjorn.Cell):
        model.remove_cell(2)
    else:
        model.remove_surface(2)
    with pytest.raises(ValueError):
        handle.id  # noqa: B018 - accessing the getter is the point


def test_removed_material_handle_raises_on_every_getter() -> None:
    model = migjorn.parse(MODEL)
    mat = model.material(1)
    assert mat is not None
    model.remove_material(1)
    with pytest.raises(ValueError):
        mat.entries  # noqa: B018 - accessing the getter is the point
    with pytest.raises(ValueError):
        mat.well_formed  # noqa: B018 - accessing the getter is the point


def test_a_stale_handle_from_before_an_unrelated_edit_still_resolves() -> None:
    """Removing one card must not invalidate handles to other cards (no
    whole-model reindex/flush hiding behind an edit)."""
    model = migjorn.parse(MODEL)
    fuel = model.cell(1)
    assert fuel is not None
    model.remove_cell(3)
    assert fuel.id == 1
    assert fuel.material == 1


# --- diagnostics content survive the boundary ---------------------------------


def test_diagnostics_content_crosses_the_boundary_intact() -> None:
    model = migjorn.parse(MALFORMED)
    messages = {d.message for d in model.diagnostics}
    assert "unrecognized token `-`" in messages
    assert "cell 4 is not well formed" in messages
    assert "surface 2 is not well formed" in messages
    assert "material 1 has an unreadable entry" in messages
    for d in model.diagnostics:
        assert d.severity in ("error", "warning")
        assert 0 <= d.start <= d.end <= len(MALFORMED)
    # malformed input must still round-trip losslessly through the bindings
    assert model.to_source() == MALFORMED


def test_well_formed_reflects_the_diagnostics() -> None:
    model = migjorn.parse(MALFORMED)
    cell = model.cell(4)
    assert cell is not None
    assert cell.well_formed is False
    surf = model.surface(2)
    assert surf is not None
    assert surf.well_formed is False
    cell = model.cell(1)
    assert cell is not None
    assert cell.well_formed is True


# --- composition: validate / merge / extract ----------------------------------


def test_validate_reports_dangling_references() -> None:
    model = migjorn.parse(
        "t\n1 1 -1.0 -99 imp:n=1\n2 0 -1 #7 fill=3 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n"
    )
    problems = model.validate()
    joined = "\n".join(problems)
    assert "undefined surface 99" in joined
    assert "undefined cell 7" in joined
    assert "undefined universe 3" in joined


def test_validate_is_empty_for_a_consistent_model() -> None:
    assert migjorn.parse(MODEL).validate() == []


def test_extract_universe_pulls_only_its_dependencies() -> None:
    model = migjorn.parse(
        "t\n"
        "1 1 -1.0 -1 u=5 imp:n=1\n"
        "2 0 -2 u=7 imp:n=1\n"
        "\n1 SO 5\n2 SO 6\n\nm1 1001 1\nm2 8016 1\n"
    )
    u5 = model.extract_universe(5)
    assert u5.num_cells == 1
    assert u5.cell(1) is not None
    assert u5.surface(2) is None
    assert u5.material(2) is None


def test_extract_level0_takes_only_rootless_cells() -> None:
    model = migjorn.parse(
        "t\n1 0 -1 imp:n=1\n2 0 -2 u=5 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n"
    )
    root = model.extract_level0()
    assert root.num_cells == 1
    assert root.cell(2) is None


def test_extract_universe_does_not_recurse_into_filled_sub_universes() -> None:
    # extract_universe pulls only universe 5's own cell and leaves the fill=
    # dangling; recursing across universe boundaries is extract_cells' job.
    model = migjorn.parse(
        "t\n"
        "1 0 -1 u=5 fill=7 imp:n=1\n"
        "2 1 -1.0 -2 u=7 imp:n=1\n"
        "\n1 SO 5\n2 SO 6\n\nm1 1001 1\n"
    )
    u5 = model.extract_universe(5)
    assert u5.num_cells == 1
    assert u5.cell(1) is not None
    assert u5.cell(2) is None


def test_extract_cells_recurses_through_fill_like_and_complement() -> None:
    model = migjorn.parse(
        "t\n"
        "1 0 -1 fill=5 imp:n=1\n"
        "2 1 -1.0 -2 u=5 imp:n=1\n"
        "3 0 -3 #2 imp:n=1\n"
        "4 LIKE 3 BUT trcl=1 imp:n=1\n"
        "\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n"
    )
    extracted = model.extract_cells([4])
    assert extracted.num_cells == 3
    assert extracted.cell(4) is not None
    assert extracted.cell(3) is not None
    assert extracted.cell(2) is not None
    assert extracted.cell(1) is None


def test_extract_cells_ignores_unknown_ids() -> None:
    model = migjorn.parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    extracted = model.extract_cells([1, 999])
    assert extracted.num_cells == 1


def test_merge_combines_disjoint_models() -> None:
    a = migjorn.parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n")
    b = migjorn.parse("t\n2 0 -2 imp:n=1\n\n2 SO 6\n\nm2 8016 1\n")
    a.merge([b])
    assert a.num_cells == 2
    assert a.num_surfaces == 2
    assert a.num_materials == 2
    assert a.cell(2) is not None
    assert a.surface(2) is not None


# --- per-field mutators through the boundary ----------------------------------


def test_surface_set_coeff_and_set_coeffs() -> None:
    model = migjorn.parse(MODEL)
    surf = model.surface(1)
    assert surf is not None
    surf.set_coeff(0, 9.0)
    assert surf.coeffs == [9.0]
    surf.coeffs = [1.0, 2.0]
    assert surf.coeffs == [1.0, 2.0]


def test_material_set_fraction_and_set_zaid() -> None:
    model = migjorn.parse(MODEL)
    mat = model.material(1)
    assert mat is not None
    mat.set_fraction(0, 0.5)
    mat.set_zaid(1, "8017.31c")
    assert mat.entries[0] == ("1001.31c", 0.5)
    assert mat.entries[1] == ("8017.31c", 0.333)


def test_transform_set_coeffs() -> None:
    model = migjorn.parse(MODEL)
    tr = model.transform(1)
    assert tr is not None
    tr.coeffs = [1.0, 2.0, 3.0]
    assert tr.coeffs == [1.0, 2.0, 3.0]


def test_add_transform_appends_and_is_visible() -> None:
    model = migjorn.parse(MODEL)
    before = model.num_transforms
    tr = model.add_transform("tr2 0 0 5")
    assert model.num_transforms == before + 1
    assert tr.id == 2
    assert model.transform(2) is not None
    assert "tr2 0 0 5" in model.to_source()


def test_data_cards_reads_generic_cards_including_sdef() -> None:
    model = migjorn.parse(MODEL)
    names = {dc.name for dc in model.data_cards()}
    assert "sdef" in names
    assert "m1" in names  # superset: Mn/TRn are Data cards too
    assert "tr1" in names
    sdef = next(dc for dc in model.data_cards() if dc.name == "sdef")
    assert sdef.particle is None
    assert sdef.starred is False
    assert "sdef" in sdef.text


def test_add_data_card_appends_and_is_readable() -> None:
    model = migjorn.parse(MODEL)
    dc = model.add_data_card("mode n")
    assert dc.name == "mode"
    assert dc.particle is None
    assert dc.starred is False
    assert "mode n" in model.to_source()


def test_data_card_set_text() -> None:
    model = migjorn.parse(MODEL)
    dc = model.add_data_card("sdef pos=0 0 0")
    dc.text = "sdef pos=1 1 1"
    assert "sdef pos=1 1 1" in dc.text
    assert "sdef pos=1 1 1" in model.to_source()


def test_data_card_remove() -> None:
    model = migjorn.parse(MODEL)
    dc = model.add_data_card("mode n")
    assert dc.remove() is True
    assert dc.remove() is False
    with pytest.raises(ValueError, match="removed"):
        _ = dc.text


def test_cell_param_lifecycle_through_bindings() -> None:
    model = migjorn.parse(MODEL)
    cell = model.cell(3)
    assert cell is not None
    assert cell.set_param("imp:n", "2") is True
    assert cell.param("imp:n").value == "2"  # type: ignore
    assert cell.set_param("nope", "1") is False
    cell.add_param("vol=7")
    assert cell.param("vol").value == "7"  # type: ignore
    assert cell.remove_param("vol") is True
    assert cell.param("vol") is None


def test_append_comment() -> None:
    model = migjorn.parse(MODEL)
    cell = model.cell(1)
    assert cell is not None
    cell.append_comment("note")
    assert "note" in model.to_source()


# --- __repr__ / __str__ -------------------------------------------------------


def test_model_repr_is_a_short_summary_and_str_is_the_full_source() -> None:
    model = migjorn.parse(MODEL)
    assert repr(model) == (
        'Model(title="Example model", 3 cells, 2 surfaces, 1 materials, 1 transforms)'
    )
    assert str(model) == model.to_source()


def test_handle_str_is_the_card_text_and_repr_shows_the_id() -> None:
    model = migjorn.parse(MODEL)
    cell = model.cell(1)
    assert repr(cell) == "Cell(id=1)"
    assert str(cell) == cell.text

    surf = model.surface(1)
    assert repr(surf) == "Surface(id=1)"
    assert str(surf) == surf.text

    mat = model.material(1)
    assert repr(mat) == "Material(id=1)"
    assert str(mat) == mat.text

    tr = model.transform(1)
    assert repr(tr) == "Transform(id=1)"
    assert str(tr) == tr.text

    dc = next(d for d in model.data_cards() if d.name == "sdef")
    assert repr(dc) == 'DataCard(name="sdef")'
    assert str(dc) == dc.text


def test_removed_handle_repr_is_safe_but_str_still_raises() -> None:
    model = migjorn.parse(MODEL)
    cell = model.cell(1)
    model.remove_cell(1)
    assert repr(cell) == "Cell(<removed>)"
    with pytest.raises(ValueError, match="removed"):
        str(cell)


# --- module-level surface -----------------------------------------------------


def test_parse_function_and_model_constructor_agree() -> None:
    a = migjorn.parse(MODEL)
    b = migjorn.Model(MODEL)
    assert a.to_source() == b.to_source() == MODEL


def test_module_exposes_version_and_merge_error() -> None:
    assert isinstance(migjorn.__version__, str)
    assert issubclass(migjorn.MergeError, ValueError)


def test_from_file_and_save_round_trip(tmp_path) -> None:
    path = tmp_path / "model.mcnp"
    path.write_text(MODEL, encoding="utf-8")
    model = migjorn.Model.from_file(str(path))
    assert model.to_source() == MODEL

    out_path = tmp_path / "out.mcnp"
    model.save(str(out_path))
    assert out_path.read_text(encoding="utf-8") == MODEL


def test_from_file_missing_path_raises_ioerror(tmp_path) -> None:
    with pytest.raises(OSError):
        migjorn.Model.from_file(str(tmp_path / "does_not_exist.mcnp"))
