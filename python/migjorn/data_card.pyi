from migjorn.card import Card

class Zaid:
    """A single nuclide entry (ZAID) within a material card."""

    isotope: int
    """ZAID nuclide identifier (e.g. 92238 for U-238)."""
    library: str
    """Cross-section library suffix (e.g. ``'70c'``)."""
    fraction: float
    """Atom or weight fraction (negative = weight fraction)."""

class MaterialCard(Card):
    """A parsed MCNP material card (Mn).

    Gives access to the material ID and its list of ``Zaid`` nuclide entries.
    """

    @property
    def card_id(self) -> int:
        """The material ID number (same as ``material_id``)."""
        ...

    @card_id.setter
    def card_id(self, new_id: int) -> None:  # type: ignore[override]
        """Set the material ID."""
        ...

    material_id: int
    """The material ID number."""

    zaids: list[Zaid]
    """List of nuclide entries that make up this material."""

    @property
    def original_text(self) -> str:
        """Return the original text of the material card as it appeared in the input
        file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the updated text of the material card after any modifications."""
        ...

class TransformCard(Card):
    """A parsed MCNP transformation card (TRn / *TRn).

    The transform ID is writable; ``starred``, ``coeffs`` are read-only.
    """

    @property
    def card_id(self) -> int:
        """The transformation ID number (same as ``transform_id``)."""
        ...

    @card_id.setter
    def card_id(self, new_id: int) -> None:  # type: ignore[override]
        """Set the transformation ID."""
        ...

    transform_id: int
    """The transformation ID number."""

    starred: bool
    """``True`` if the card uses the starred form (*TR), meaning angles are in
    degrees instead of direction cosines. Read-only."""

    coeffs: list[float]
    """Transformation coefficients (displacement vector and rotation matrix).
    Read-only."""

    @property
    def original_text(self) -> str:
        """Return the original text of the transformation card as it appeared in the
        input file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the updated text of the transformation card after any modifications."""
        ...

class TallyCard(Card):
    """A parsed MCNP tally card (Fn).

    The tally ID is writable; the rest of the fields are read-only.
    The associated cards to a tally are treated as other independent tally cards. That is,
    FMESH4, FC4, FM4 are all separate tally cards that happen to have the same tally ID.
    """

    @property
    def card_id(self) -> int:
        """The tally ID number (same as ``tally_id``)."""
        ...

    @card_id.setter
    def card_id(self, new_id: int) -> None:  # type: ignore[override]
        """Set the tally ID."""
        ...

    tally_id: int
    """The tally ID number."""

    @property
    def original_text(self) -> str:
        """Return the original text of the tally card as it appeared in the input file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the updated text of the tally card after any modifications."""
        ...

class UnparsedCard(Card):
    """A data card that migjorn does not parse in detail.

    The raw card name is accessible; the original bytes are preserved on write.
    """

    card_name: str
    """The card keyword/name as it appears in the input file. Read-only."""

    @property
    def card_id(self) -> str:
        """The card ID (same as ``card_name``)."""
        ...

    @card_id.setter
    def card_id(self, new_id: str) -> None:  # type: ignore[override]
        """Cannot change the card ID of an UnparsedCard."""
        raise AttributeError("Cannot change the card ID of an UnparsedCard")

    @property
    def original_text(self) -> str:
        """Return the original text of the card as it appeared in the input file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the updated text of the card after any modifications."""
        ...
