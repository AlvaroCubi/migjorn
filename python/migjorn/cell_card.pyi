from migjorn.card import Card

class CellCard(Card):
    """A parsed MCNP cell card.

    Wraps the cell definition from an input file, giving read/write access to
    the cell ID, material, density, geometry, and cell parameters (IMP, U, etc.).
    Modifying a field updates the in-memory representation; call
    ``Model.write_to_file`` to persist the changes.
    """

    @property
    def card_id(self) -> int:
        """The cell ID number (same as ``cell_id``)."""
        ...

    @card_id.setter
    def card_id(self, new_id: int) -> None:  # type: ignore[override]
        """Set the cell ID."""
        ...

    material_id: int
    """The material number referenced by this cell (0 = void)."""

    density: float | None
    """Atom density (positive value) or mass density (negative value)of the cell
    (``None`` for void cells).
    """

    geometry: list[str]
    """Geometry definition as a list of tokens: surface IDs and operator
    characters (e.g. ``["-1", ":", "2"]``). Read-only."""

    params: list[CellParameter]
    """Cell parameters (IMP:N, VOL, FILL, etc.) as a list of
    :class:`CellParameter` objects. Iterate to inspect or modify."""

    @property
    def original_text(self) -> str:
        """Return the original text of the cell card as it appeared in the input
        file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the updated text of the cell card after any modifications."""
        ...

    def remove_param(self, index: int) -> None:
        """Remove the parameter at the given index."""
        ...

    def insert_parameter(self, text: str, index: int | None = None) -> None:
        """Parse ``text`` as a cell parameter (e.g. ``"IMP:N=1.0"``) and insert
        it at ``index`` (default: append at end).

        Raises ``ValueError`` if ``text`` cannot be parsed as a known parameter.
        """
        ...

    def remove_geometry_element(self, index: int) -> None:
        """Remove the geometry element at the given index."""
        ...

    def insert_geometry_elements(self, text: str, index: int | None = None) -> None:
        """Parse ``text`` (e.g. ``"-10"``, ``":"``, ``"#40"``) and insert the
        resulting geometry element(s) at ``index`` (default: append).

        ``"#40"`` produces two elements (complement operator + cell ID) inserted
        consecutively. Raises ``ValueError`` if ``text`` cannot be parsed.
        """
        ...

    def void_cell(self) -> None:
        """Set this cell as void (material ID 0 and no density)."""
        ...

class CellParameter:
    """A single cell parameter (IMP:N, VOL, FILL, etc.).

    Obtained by iterating ``cell.params``. Mutations are reflected immediately
    in the parent model.
    """

    param_type: str
    """The parameter keyword: one of ``"IMP:N"``, ``"IMP:P"``, ``"U"``,
    ``"VOL"``, ``"TMP"``, ``"LAT"``, ``"FILL"``, ``"*FILL"``."""

    value: float | int
    """The primary scalar value. Float for IMP, VOL, TMP; int for U, LAT, FILL
    (universe ID). Setting this updates the model in-place."""

    fill_starred: bool | None
    """``True`` for ``*FILL`` (rotation-matrix form). ``None`` for non-FILL params."""

    fill_transform: int | None
    """The transformation ID inside the FILL parentheses, or ``None``."""

    fill_coeffs: list[float]
    """Coefficient list for ``*FILL`` rotation matrices. Empty for other params."""
