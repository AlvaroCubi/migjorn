import tempfile
from pathlib import Path

from migjorn import Model, run

TRACKED = Path(__file__).parent / "../../resources/simple_model.mcnp"


def test_cli_info_valid_file():
    assert run(["migjorn", "info", str(TRACKED)]) == 0


def test_cli_info_missing_file_returns_1():
    assert run(["migjorn", "info", "/dev/null/does_not_exist.mcnp"]) == 1


def test_cli_parsing_check_valid_file():
    assert run(["migjorn", "parsing-check", str(TRACKED)]) == 0


def test_cli_parsing_check_missing_file_returns_1():
    assert run(["migjorn", "parsing-check", "/dev/null/does_not_exist.mcnp"]) == 1


def test_cli_renumber_cells():
    with tempfile.NamedTemporaryFile(suffix=".mcnp", delete=False) as tmp:
        result = run(["migjorn", "renumber", str(TRACKED), tmp.name, "--cells", "1000"])
        assert result == 0
        model = Model.from_file(tmp.name)
        cell_ids = sorted(c.card_id for c in model.cells)
        assert cell_ids == [1001, 1002, 1003, 1004, 1005]
