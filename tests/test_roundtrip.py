"""Losslessness contract: every corpus file must round-trip byte-for-byte.

This is data-driven — it discovers every ``*.mcnp`` under ``corpus/`` and needs
no per-file configuration. Drop a new fixture in and it is automatically held to
the invariant. This mirrors the single most important promise of the library:

    parse(text).to_source() == text          # exactly, including the malformed
                                              # fixture, BOM, and trailing space

Run: ``pytest tests/test_roundtrip.py -q`` (once ``migjorn`` is built).
"""

from __future__ import annotations

from pathlib import Path

import migjorn
import pytest

CORPUS = Path(__file__).parent / "corpus"
FIXTURES = sorted(CORPUS.glob("*.mcnp"))
IDS = [f.stem for f in FIXTURES]


@pytest.mark.parametrize("path", FIXTURES, ids=IDS)
def test_roundtrip_bytes(path: Path) -> None:
    original = path.read_bytes()
    model = migjorn.parse(original.decode("utf-8"))
    assert model.to_source().encode("utf-8") == original


@pytest.mark.parametrize("path", FIXTURES, ids=IDS)
def test_parsing_never_raises(path: Path) -> None:
    """Recoverability: even malformed input yields a model + diagnostics, never
    an exception. Malformed fixtures still round-trip losslessly."""
    text = path.read_text(encoding="utf-8")
    model = migjorn.parse(text)  # must not raise
    assert isinstance(model.to_source(), str)


def test_edit_is_lossless_except_where_changed() -> None:
    """A single edit changes only the edited bytes; everything else is intact."""
    src = (CORPUS / "minimal.mcnp").read_text(encoding="utf-8")
    model = migjorn.parse(src)
    cell = model.cell(1)
    assert cell is not None
    cell.material = 2  # 1 1 -1.0 ... → 1 2 -1.0 ...
    out = model.to_source()
    assert "1 2 -1.0 -1 imp:n=1 $ fuel sphere" in out
    assert "$ fuel sphere" in out  # inline comment preserved
    assert out.count("\n") == src.count("\n")  # no line added/removed
