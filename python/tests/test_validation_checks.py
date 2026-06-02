import pytest
from migjorn import Model

VALID = """Title
1 0 -1 imp:n=1

1 SO 10

SDFEF pos 0 0 0
"""

INVALID = """Title
1 0 -1 imp:n=1

2 SO 10

SDFEF pos 0 0 0
"""

MISSING_TRANSFORM_FILL = """Title
1 1 -1 -10 FILL=5 (99) IMP:N=1
2 0 10 IMP:N=0

10 PX 0

M1 1001.32c 1.0
"""

MISSING_TRANSFORM_SURFACE = """Title
1 1 -1 -10 IMP:N=1
2 0 10 IMP:N=0

10 99 PX 0

M1 1001.32c 1.0
"""

MISSING_CELL_COMPLEMENT = """Title
1 1 -1 -10 IMP:N=1
2 0 #99 IMP:N=0

10 PX 0

M1 1001.32c 1.0
"""


def test_validation_checks():
    model = Model(VALID)
    result = model.validation_checks()  # Should not raise
    assert result is None


def test_validation_checks_with_errors():
    model = Model(INVALID)
    with pytest.raises(ValueError) as exc_info:
        model.validation_checks()
    assert "Error: Missing surface IDs referenced in geometry: {1}\n" in str(
        exc_info.value
    )


def test_missing_transform_via_fill_is_detected():
    model = Model(MISSING_TRANSFORM_FILL)
    with pytest.raises(ValueError) as exc_info:
        model.validation_checks()
    assert "Missing transform IDs" in str(exc_info.value)
    assert "99" in str(exc_info.value)


def test_missing_transform_via_surface_is_detected():
    model = Model(MISSING_TRANSFORM_SURFACE)
    with pytest.raises(ValueError) as exc_info:
        model.validation_checks()
    assert "Missing transform IDs" in str(exc_info.value)
    assert "99" in str(exc_info.value)


def test_missing_cell_in_complement_is_detected():
    model = Model(MISSING_CELL_COMPLEMENT)
    with pytest.raises(ValueError) as exc_info:
        model.validation_checks()
    assert "Missing cell IDs" in str(exc_info.value)
    assert "99" in str(exc_info.value)
